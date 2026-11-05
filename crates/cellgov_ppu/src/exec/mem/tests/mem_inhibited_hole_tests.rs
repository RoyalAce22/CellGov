//! A load the PPE refuses in caching-inhibited storage that runs out of
//! its view into unmapped memory faults on the first unmapped byte.
//!
//! [CBE-Handbook p:290 s:9.10] A load's page-fault data storage interrupt precedes its alignment interrupt in caching-inhibited storage.

use super::caching_inhibited::{refused_forms, run, view, BASE, EA};
use super::*;

/// The first byte past the caching-inhibited view.
const HOLE: u64 = EA + 1;

#[test]
fn each_refused_load_that_runs_into_a_hole_faults_on_the_first_unmapped_byte() {
    let mem = [0xA5u8; 0x200];
    let views = [view(&mem[..(HOLE - BASE) as usize], true, None)];
    // The loads whose operand starts in the view; an lvrx's operand is
    // the part of the quadword below its EA.
    let loads: Vec<_> = refused_forms()
        .into_iter()
        .filter(|&(insn, ea, store)| {
            let start = match insn {
                PpuInstruction::Lvrx { .. } => ea & !15,
                _ => ea,
            };
            !store && start < HOLE
        })
        .collect();
    assert_eq!(loads.len(), 8);
    for (insn, _, _) in loads {
        let (v, _, effects) = run(insn, &views);
        assert_eq!(
            v,
            ExecuteVerdict::MemFault(crate::exec::memory_helpers::unmapped(HOLE)),
            "{insn:?}"
        );
        assert!(effects.is_empty(), "{insn:?}: {effects:?}");
    }
}

#[test]
fn a_fused_pair_whose_unaligned_half_runs_into_a_hole_faults_on_the_hole() {
    let mem = [0xA5u8; 0x200];
    let views = [view(&mem[..(EA + 4 - BASE) as usize], true, None)];
    let pair = crate::instruction::FusedPair::LwzLwz {
        rt1: 5,
        ra1: 3,
        off1: 0,
        rt2: 6,
        ra2: 3,
        off2: 2,
    };
    let (v, _, _) = run(PpuInstruction::Fused(pair), &views);
    assert_eq!(
        v,
        ExecuteVerdict::MemFault(crate::exec::memory_helpers::unmapped(EA + 4))
    );
}

/// Runs `insn` over `views` with r3 = [`EA`], r4 = 2 and the XER byte
/// count `count`.
fn run_counted(
    insn: PpuInstruction,
    views: &[cellgov_mem::RegionView<'_>],
    count: u64,
) -> (ExecuteVerdict, Vec<Effect>) {
    let mut s = PpuState::new();
    s.set_gpr(3, EA);
    s.set_gpr(4, 2);
    s.set_xer(count);
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    let v = execute(&insn, &mut s, uid(), views, &mut effects, &mut store_buf);
    store_buf.flush(&mut effects, uid());
    (v, effects)
}

#[test]
fn an_unaligned_lwz_an_lvlx_and_an_lswx_that_run_into_a_hole_fault_on_the_hole() {
    let mem = [0xA5u8; 0x200];
    let views = [view(&mem[..(EA + 4 - BASE) as usize], true, None)];
    let loads = [
        // [EA + 1, EA + 5)
        PpuInstruction::Lwz {
            rt: 5,
            ra: 3,
            imm: 1,
        },
        // [EA + 2, EA + 16)
        PpuInstruction::Lvlx {
            vt: 1,
            ra: 3,
            rb: 4,
        },
        // [EA + 2, EA + 10)
        PpuInstruction::Lswx {
            rt: 5,
            ra: 3,
            rb: 4,
        },
    ];
    for insn in loads {
        let (v, effects) = run_counted(insn, &views, 8);
        assert_eq!(
            v,
            ExecuteVerdict::MemFault(crate::exec::memory_helpers::unmapped(EA + 4)),
            "{insn:?}"
        );
        assert!(effects.is_empty(), "{insn:?}: {effects:?}");
    }
}

#[test]
fn a_refused_load_that_ends_at_the_end_of_its_view_takes_the_alignment_interrupt() {
    let mem = [0xA5u8; 0x200];
    // An lwarx of [EA, EA + 4) and an lvrx of [EA, EA + 2) with EA + 2 in r3 + r4.
    let cases = [
        (
            PpuInstruction::Lwarx {
                rt: 5,
                ra: 0,
                rb: 3,
            },
            EA + 4,
            EA,
        ),
        (
            PpuInstruction::Lvrx {
                vt: 1,
                ra: 3,
                rb: 4,
            },
            EA + 2,
            EA + 2,
        ),
    ];
    for (insn, end, dar) in cases {
        let views = [view(&mem[..(end - BASE) as usize], true, None)];
        let (v, _) = run_counted(insn, &views, 0);
        assert_eq!(
            v,
            ExecuteVerdict::Fault(PpuFault::alignment(dar, false)),
            "{insn:?}"
        );
    }
}

#[test]
fn a_refused_load_held_by_its_view_still_takes_the_alignment_interrupt() {
    let mem = [0xA5u8; 0x200];
    let views = [view(&mem, true, None)];
    let lwz = PpuInstruction::Lwz {
        rt: 5,
        ra: 3,
        imm: 1,
    };
    let (v, _, _) = run(lwz, &views);
    assert_eq!(v, ExecuteVerdict::Fault(PpuFault::alignment(EA + 1, false)));
}
