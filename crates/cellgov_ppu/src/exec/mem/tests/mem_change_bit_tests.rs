//! A store the PPE refuses in caching-inhibited storage that a view also
//! refuses orders the two refusals by the page table entry's C bit.
//!
//! [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1, and follows it when C is 0.

use super::caching_inhibited::{refused_forms, run, view, BASE, EA};
use super::*;

/// Each store form the PPE refuses in caching-inhibited storage, with
/// the EA its alignment interrupt names.
fn refused_stores() -> Vec<(PpuInstruction, u64)> {
    let mut stores: Vec<_> = refused_forms()
        .into_iter()
        .filter(|&(_, _, store)| store)
        .map(|(insn, ea, _)| (insn, ea))
        .collect();
    stores.push((PpuInstruction::Dcbz { ra: 0, rb: 3 }, EA));
    stores
}

#[test]
fn each_refused_store_to_a_read_only_view_takes_the_protection_when_c_is_0() {
    let mem = [0u8; 0x200];
    let views = [view(&mem, true, Some("page protection"))];
    for (insn, _) in refused_stores() {
        let (v, _, effects) = run(insn, &views);
        assert!(
            matches!(
                v,
                ExecuteVerdict::StoreFault(cellgov_mem::MemError::ReservedWrite { .. })
            ),
            "{insn:?}: {v:?}"
        );
        assert!(effects.is_empty(), "{insn:?}: {effects:?}");
    }
}

/// [CBE-Handbook p:290 s:9.10] The page-fault data storage interrupt precedes a caching-inhibited store's alignment interrupt and its page-protection interrupt.
#[test]
fn a_store_from_a_read_only_view_with_c_1_into_no_view_faults_on_the_first_unheld_byte() {
    let mem = [0u8; 0x200];
    let views = [cellgov_mem::RegionView {
        store_alignment_first: true,
        ..view(&mem, true, Some("page protection"))
    }];
    // From EA + 0x180, the stmw's 128 bytes run 0x40 bytes past the view.
    let insn = PpuInstruction::Stmw {
        rs: 0,
        ra: 3,
        imm: 0x180,
    };
    let (v, _, effects) = run(insn, &views);
    let ExecuteVerdict::StoreFault(cellgov_mem::MemError::Unmapped(ctx)) = v else {
        panic!("{v:?}");
    };
    assert_eq!(ctx.addr, BASE + 0x200);
    assert!(effects.is_empty(), "{effects:?}");
}

#[test]
fn each_refused_store_to_a_read_only_view_takes_the_alignment_interrupt_when_c_is_1() {
    let mem = [0u8; 0x200];
    let views = [cellgov_mem::RegionView {
        store_alignment_first: true,
        ..view(&mem, true, Some("page protection"))
    }];
    for (insn, ea) in refused_stores() {
        let (v, _, effects) = run(insn, &views);
        assert_eq!(
            v,
            ExecuteVerdict::Fault(PpuFault::alignment(ea, true)),
            "{insn:?}"
        );
        assert!(effects.is_empty(), "{insn:?}: {effects:?}");
    }
}
