//! Which loads and stores a batch marks caching-inhibited for the timing
//! model, and what the timing model's caches hold after them.
//!
//! [CBE-Handbook p:134 s:6.1.2.2] The CIU sends an access to the NCU or to the L2 by the page's I bit.

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
/// An effective page that a translation maps onto [`CACHED`].
const PAGE_EA: u64 = 0xC000_0000;
/// Where a batch test's code starts.
const CODE: u64 = 0x1000;

static BYTES: [u8; PAGE as usize] = [0; PAGE as usize];

/// `stw r8,0(r3)`.
const STW: I = I::Stw {
    rs: 8,
    ra: 3,
    imm: 0,
};
/// `lwz r4,0(r3)`.
const LWZ: I = I::Lwz {
    rt: 4,
    ra: 3,
    imm: 0,
};

/// The caching-inhibited flag of the one op of `insn`, with r3 at `ea`,
/// after `table` marks it.
fn marked(table: &RegionTable<'_>, insn: I, ea: u64) -> bool {
    let mut state = PpuState::new();
    state.set_gpr(3, ea);
    let (mut ops, n) = classify(&insn, &state);
    assert_eq!(n, 1);
    table.mark_inhibited(&mut ops[..n]);
    match ops[0].kind {
        Kind::Load { inhibited, .. } | Kind::Store { inhibited, .. } => inhibited,
        other => panic!("{other:?} makes no load or store"),
    }
}

#[test]
fn a_load_or_store_in_a_caching_inhibited_view_is_marked_and_one_in_a_cacheable_view_is_not() {
    let inhibited = RegionView {
        caching_inhibited: true,
        ..RegionView::plain(INHIBITED, &BYTES)
    };
    let table = RegionTable::new([RegionView::plain(CACHED, &BYTES), inhibited].into_iter());
    for insn in [STW, LWZ] {
        assert!(marked(&table, insn, INHIBITED), "{insn:?}");
        assert!(!marked(&table, insn, CACHED), "{insn:?}");
    }
}

/// A unit with one data translation of [`PAGE_EA`] onto [`CACHED`],
/// whose page has the I bit `inhibited`.
fn translating_unit(inhibited: bool) -> PpuExecutionUnit {
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.translations.insert(
        Access::Load,
        Translation {
            ea: PAGE_EA,
            ra: CACHED,
            len: PAGE,
            writable: true,
            inhibited,
            guarded: false,
            changed: true,
        },
    );
    unit
}

#[test]
fn a_load_or_store_through_a_page_with_i_set_over_cacheable_storage_is_marked() {
    let mem = GuestMemory::new(0x2_0000);
    for inhibited in [true, false] {
        let unit = translating_unit(inhibited);
        let table = RegionTable::new(unit.translated_views(&mem).into_iter());
        for insn in [STW, LWZ] {
            assert_eq!(
                marked(&table, insn, PAGE_EA),
                inhibited,
                "{insn:?} with I = {inhibited}"
            );
        }
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

/// Whether the timing model's L1 and L2 hold the line at `ea` after a
/// thread that does not translate runs `insn` with r3 at `ea`.
fn held_after(insn: I, ea: u64) -> (bool, bool) {
    let mem = memory(insn);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().pc = CODE;
    unit.state_mut().set_gpr(3, ea);
    let r = unit.run_until_yield(
        Budget::new(1),
        &ExecutionContext::new(&mem),
        &mut Vec::new(),
    );
    assert!(r.fault.is_none(), "{insn:?} takes no fault: {:?}", r.fault);
    assert_eq!(unit.state().pc, CODE + 4, "{insn:?} retires");
    unit.timing.caches().holds(ea, ea)
}

#[test]
fn a_batch_store_to_a_caching_inhibited_region_leaves_no_line_in_the_caches() {
    assert_eq!(held_after(STW, INHIBITED), (false, false));
    assert_eq!(held_after(STW, CACHED), (false, true));
}

#[test]
fn a_batch_load_from_a_caching_inhibited_region_leaves_no_line_in_the_caches() {
    assert_eq!(held_after(LWZ, INHIBITED), (false, false));
    assert_eq!(held_after(LWZ, CACHED), (true, true));
}
