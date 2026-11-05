//! dcbz into caching-inhibited storage, such as the RSX window.

use super::*;
use cellgov_ps3_abi::hw::address_space::{PS3_RSX_BASE, PS3_RSX_SIZE};

/// Runs one dcbz at `target` over `mem` mapped at `base`, in a view that
/// is caching-inhibited when `inhibited` is set.
fn dcbz(
    target: u64,
    base: u64,
    mem: &[u8],
    inhibited: bool,
) -> (ExecuteVerdict, PpuState, Vec<Effect>) {
    let views = [cellgov_mem::RegionView {
        caching_inhibited: inhibited,
        ..cellgov_mem::RegionView::plain(base, mem)
    }];
    dcbz_over(target, &views)
}

/// Runs one dcbz at `target` over `views`.
fn dcbz_over(
    target: u64,
    views: &[cellgov_mem::RegionView<'_>],
) -> (ExecuteVerdict, PpuState, Vec<Effect>) {
    let mut s = PpuState::new();
    s.set_gpr(3, target);
    s.set_gpr(4, 0);
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    let v = execute(
        &PpuInstruction::Dcbz { ra: 3, rb: 4 },
        &mut s,
        uid(),
        views,
        &mut effects,
        &mut store_buf,
    );
    store_buf.flush(&mut effects, uid());
    (v, s, effects)
}

/// The address and bytes of each write the effects carry, in order.
fn written(effects: &[Effect]) -> Vec<(u64, Vec<u8>)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                Some((range.start().raw(), bytes.bytes().to_vec()))
            }
            _ => None,
        })
        .collect()
}

/// The 16 zero doublewords of the 128-byte block at `block`.
fn zeroed(block: u64) -> Vec<(u64, Vec<u8>)> {
    (0..0x80u64)
        .step_by(8)
        .map(|off| (block + off, vec![0u8; 8]))
        .collect()
}

/// [CBE-Handbook p:253 s:9.5.8] A dcbz to caching-inhibited storage takes
/// an alignment interrupt; the RSX window is caching-inhibited.
/// [PPC-Book3 p:70 s:5.5.8] The DAR holds the effective address the
/// instruction computed.
#[test]
fn a_dcbz_into_the_rsx_window_takes_the_alignment_fault() {
    let target = PS3_RSX_BASE + 0x40;
    let (v, s, effects) = dcbz(target, PS3_RSX_BASE, &[0xEE; 0x200], true);
    assert_eq!(v, ExecuteVerdict::Fault(PpuFault::alignment(target, true)));
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(s.dcbz_executed, 0);
}

/// Runs one dcbz at `target` over four adjacent 128-byte views:
///
/// - a cacheable block just below the RSX window
/// - the window's first block, caching-inhibited
/// - the window's last block, caching-inhibited
/// - a cacheable block just above the window
fn dcbz_at_the_rsx_edges(target: u64) -> (ExecuteVerdict, PpuState, Vec<Effect>) {
    let rsx_end = PS3_RSX_BASE + PS3_RSX_SIZE as u64;
    let blocks = [[0xEEu8; 0x80]; 4];
    let views = [
        cellgov_mem::RegionView::plain(PS3_RSX_BASE - 0x80, &blocks[0]),
        cellgov_mem::RegionView {
            caching_inhibited: true,
            ..cellgov_mem::RegionView::plain(PS3_RSX_BASE, &blocks[1])
        },
        cellgov_mem::RegionView {
            caching_inhibited: true,
            ..cellgov_mem::RegionView::plain(rsx_end - 0x80, &blocks[2])
        },
        cellgov_mem::RegionView::plain(rsx_end, &blocks[3]),
    ];
    dcbz_over(target, &views)
}

/// [PPC-Book3 p:70 s:5.5.8] The DAR holds the effective address the
/// instruction computed, also when it is not block-aligned.
#[test]
fn a_dcbz_at_the_first_or_last_byte_of_the_rsx_window_takes_the_alignment_fault() {
    for target in [PS3_RSX_BASE, PS3_RSX_BASE + PS3_RSX_SIZE as u64 - 1] {
        let (v, s, effects) = dcbz_at_the_rsx_edges(target);
        assert_eq!(
            v,
            ExecuteVerdict::Fault(PpuFault::alignment(target, true)),
            "{target:#x}"
        );
        assert!(effects.is_empty(), "{effects:?}");
        assert_eq!(s.dcbz_executed, 0, "{target:#x}");
    }
}

/// An untranslated page has no caching attribute.
/// [CBE-Handbook p:290 s:9.10] A page-fault data storage interrupt ranks
/// above an alignment interrupt for caching-inhibited storage.
/// [PPC-Book3 p:67 s:5.5.3] For a cache-management instruction the DAR
/// holds a byte in the block.
#[test]
fn a_dcbz_into_an_unbacked_part_of_the_rsx_window_faults_as_unmapped() {
    let target = PS3_RSX_BASE + 0x0F00_0040;
    let (v, s, effects) = dcbz(target, PS3_RSX_BASE, &[0xEE; 0x200], true);
    assert_eq!(
        v,
        ExecuteVerdict::StoreFault(crate::exec::memory_helpers::unmapped(target & !0x7F))
    );
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(s.dcbz_executed, 0);
}

/// [CBE-Handbook p:147 s:6.1.6.4] dcbz clears every byte of the 128-byte
/// line that holds the effective address.
#[test]
fn a_dcbz_on_either_side_of_the_rsx_window_zeroes_its_block() {
    let above = PS3_RSX_BASE + PS3_RSX_SIZE as u64;
    for (target, block) in [(PS3_RSX_BASE - 1, PS3_RSX_BASE - 0x80), (above, above)] {
        let (v, s, effects) = dcbz_at_the_rsx_edges(target);
        assert_eq!(v, ExecuteVerdict::Continue, "{target:#x}");
        assert_eq!(written(&effects), zeroed(block), "{target:#x}");
        assert_eq!(s.dcbz_executed, 1, "{target:#x}");
    }
}

#[test]
fn a_dcbz_at_an_rsx_address_in_a_cacheable_view_zeroes_its_block() {
    let target = PS3_RSX_BASE + 0x40;
    let (v, s, effects) = dcbz(target, PS3_RSX_BASE, &[0xEE; 0x200], false);
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(written(&effects), zeroed(PS3_RSX_BASE));
    assert_eq!(s.dcbz_executed, 1);
}

/// [CBE-Handbook p:147 s:6.1.6.4] Any caching-inhibited storage refuses a
/// dcbz, wherever it lies.
#[test]
fn a_dcbz_into_a_caching_inhibited_view_outside_the_rsx_window_takes_the_alignment_fault() {
    let target = 0x0001_0040;
    let (v, s, effects) = dcbz(target, 0x0001_0000, &[0xEE; 0x200], true);
    assert_eq!(v, ExecuteVerdict::Fault(PpuFault::alignment(target, true)));
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(s.dcbz_executed, 0);
}

const NEIGHBOUR_CACHEABLE: u64 = 0x0001_0000;
const NEIGHBOUR_INHIBITED: u64 = 0x0001_0100;

/// Runs one dcbz at `target` over a cacheable view followed by a
/// caching-inhibited one, each 0x100 bytes. The runtime hands the PPU
/// every region at once.
fn dcbz_beside(target: u64) -> (ExecuteVerdict, PpuState, Vec<Effect>) {
    let (low, high) = ([0xEEu8; 0x100], [0xEEu8; 0x100]);
    let views = [
        cellgov_mem::RegionView::plain(NEIGHBOUR_CACHEABLE, &low),
        cellgov_mem::RegionView {
            caching_inhibited: true,
            ..cellgov_mem::RegionView::plain(NEIGHBOUR_INHIBITED, &high)
        },
    ];
    dcbz_over(target, &views)
}

#[test]
fn a_dcbz_in_a_cacheable_view_beside_an_inhibited_one_zeroes_its_block() {
    let (v, s, effects) = dcbz_beside(NEIGHBOUR_CACHEABLE + 0x40);
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(written(&effects), zeroed(NEIGHBOUR_CACHEABLE));
    assert_eq!(s.dcbz_executed, 1);
}

/// [CBE-Handbook p:147 s:6.1.6.4] A dcbz to caching-inhibited storage
/// takes an alignment interrupt.
#[test]
fn a_dcbz_in_an_inhibited_view_after_a_cacheable_one_takes_the_alignment_fault() {
    let target = NEIGHBOUR_INHIBITED + 0x40;
    let (v, s, effects) = dcbz_beside(target);
    assert_eq!(v, ExecuteVerdict::Fault(PpuFault::alignment(target, true)));
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(s.dcbz_executed, 0);
}
