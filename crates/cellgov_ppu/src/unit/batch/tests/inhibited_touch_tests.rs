//! Which touches a batch cancels for caching-inhibited storage, and what
//! the timing model's caches hold after them.
//!
//! [CBE-Handbook p:145 s:6.1.6.2] A touch of a caching-inhibited page is cancelled, and the touch reloads no cache.
//!
//! [CBE-Handbook p:145 s:6.1.6.2] A touch with TH = '0000' that the PPE does not cancel reloads the line into the L2 and the L1.

use super::RegionTable;
use crate::instruction::encode::encode;
use crate::instruction::PpuInstruction as I;
use crate::mmu::{Access, Translation};
use crate::state::PpuState;
use crate::timing::{classify, Kind};
use crate::PpuExecutionUnit;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region, RegionView};
use cellgov_time::Budget;

const PAGE: u64 = 0x1000;
/// A cacheable page, and the caching-inhibited page after it.
const CACHED: u64 = 0x1_0000;
const INHIBITED: u64 = CACHED + PAGE;
/// The offset of the touched line in its page.
const LINE: u64 = 0x80;
/// An effective page that a translation maps onto [`INHIBITED`].
const PAGE_EA: u64 = 0xC000_0000;
/// Where a batch test's code starts.
const CODE: u64 = 0x1000;

static BYTES: [u8; PAGE as usize] = [0; PAGE as usize];

/// `dcbt 0,r7` with TH = '0000'.
const DCBT: I = I::Dcbt {
    ra: 0,
    rb: 7,
    th: 0,
};
/// `dcbtst 0,r7`.
const DCBTST: I = I::Dcbtst {
    ra: 0,
    rb: 7,
    th: 0,
};

/// Whether the one touch of `insn`, with r7 at `ea`, still reloads its
/// line after `table` marks it.
fn reloads(table: &RegionTable<'_>, insn: I, ea: u64) -> bool {
    let mut state = PpuState::new();
    state.set_gpr(7, ea);
    let (mut ops, n) = classify(&insn, &state);
    assert_eq!(n, 1);
    assert_eq!(ops[0].touch_ea(), Some(ea), "{insn:?} is a live touch");
    table.mark_inhibited(&mut ops[..n]);
    match ops[0].kind {
        Kind::Touch { ra, .. } => ra.is_some(),
        other => panic!("{other:?} is no touch"),
    }
}

#[test]
fn a_touch_in_a_caching_inhibited_view_is_cancelled_and_one_in_a_cacheable_view_is_not() {
    let inhibited = RegionView {
        caching_inhibited: true,
        ..RegionView::plain(INHIBITED, &BYTES)
    };
    let table = RegionTable::new([RegionView::plain(CACHED, &BYTES), inhibited].into_iter());
    for insn in [DCBT, DCBTST] {
        assert!(!reloads(&table, insn, INHIBITED + LINE), "{insn:?}");
        assert!(reloads(&table, insn, CACHED + LINE), "{insn:?}");
    }
}

/// Memory with a cacheable region at 0 and a caching-inhibited region at
/// [`INHIBITED`], and `insn` at [`CODE`].
fn memory(insn: I) -> GuestMemory {
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, INHIBITED as usize, "main", PageSize::Page4K),
        Region::new(INHIBITED, PAGE as usize, "inhibited", PageSize::Page4K)
            .with_caching_inhibited(),
    ])
    .expect("the regions do not overlap");
    let word = encode(&insn).expect("the test instruction encodes");
    let range = ByteRange::new(GuestAddr::new(CODE), 4).expect("a range");
    mem.apply_commit(range, &word.to_be_bytes())
        .expect("the memory holds the code");
    mem
}

#[test]
fn a_touch_through_a_page_with_i_clear_over_a_caching_inhibited_region_is_cancelled() {
    let mem = memory(DCBT);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.translations.insert(
        Access::Load,
        Translation {
            ea: PAGE_EA,
            ra: INHIBITED,
            len: PAGE,
            writable: true,
            inhibited: false,
            guarded: false,
            changed: true,
        },
    );
    let table = RegionTable::new(unit.translated_views(&mem).into_iter());
    for insn in [DCBT, DCBTST] {
        assert!(!reloads(&table, insn, PAGE_EA + LINE), "{insn:?}");
    }
}

/// Whether the timing model's L1 and L2 hold the line at `ea` after a
/// thread that does not translate runs `insn` with r7 at `ea`, and the
/// cycle at which the timing model can send its next touch request.
fn after_batch(insn: I, ea: u64) -> ((bool, bool), u64) {
    let mem = memory(insn);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().pc = CODE;
    unit.state_mut().set_gpr(7, ea);
    let r = unit.run_until_yield(
        Budget::new(1),
        &ExecutionContext::new(&mem),
        &mut Vec::new(),
    );
    assert!(r.fault.is_none(), "{insn:?} takes no fault: {:?}", r.fault);
    assert_eq!(unit.state().pc, CODE + 4, "{insn:?} retires");
    let caches = unit.timing.caches();
    (caches.holds(ea, ea), caches.touch_free())
}

#[test]
fn a_batch_touch_of_a_caching_inhibited_region_reloads_no_line() {
    for insn in [DCBT, DCBTST] {
        let (held, touch_free) = after_batch(insn, INHIBITED + LINE);
        assert_eq!(held, (false, false), "{insn:?} holds no line");
        assert_eq!(touch_free, 0, "{insn:?} sends no touch request");
        let (held, touch_free) = after_batch(insn, CACHED + LINE);
        assert_eq!(held, (true, true), "{insn:?} reloads the L2 and the L1");
        assert_ne!(touch_free, 0, "{insn:?} sends a touch request");
    }
}
