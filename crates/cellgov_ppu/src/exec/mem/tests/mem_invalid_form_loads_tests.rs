//! An invalid-form load multiple or load string runs the loads ahead of
//! its colliding register first, so a byte there that no view holds takes
//! the data storage interrupt before the program interrupt.
//!
//! [CBE-Handbook p:254 s:9.5.9] For an RA in the range, the PPE completes the loads up to the collision and takes the interrupt there.

use super::*;

/// Runs `insn` over `len` bytes of memory at 0 with r1..r31 holding
/// `0x100 + 4 * r` and the XER byte count `count`.
fn run(insn: PpuInstruction, len: usize, count: u64) -> (ExecuteVerdict, Vec<Effect>) {
    let mut s = PpuState::new();
    for r in 1..32 {
        s.set_gpr(r, 0x100 + 4 * r as u64);
    }
    s.set_xer(count);
    let mut effects = Vec::new();
    let mem = vec![0xAAu8; len];
    let v = exec_with_mem(&insn, &mut s, 0, &mem, &mut effects);
    (v, effects)
}

/// `lmw r20, 0(r26)`: the 24 bytes from r26 = 0x168 fill r20..r25
/// ahead of the collision.
const LMW: PpuInstruction = PpuInstruction::Lmw {
    rt: 20,
    ra: 26,
    imm: 0,
};

#[test]
fn an_invalid_lmw_whose_words_ahead_of_ra_run_into_a_hole_faults_on_the_hole() {
    let (v, effects) = run(LMW, 0x170, 0);
    assert_eq!(
        v,
        ExecuteVerdict::MemFault(crate::exec::memory_helpers::unmapped(0x170))
    );
    assert!(effects.is_empty(), "{effects:?}");
}

#[test]
fn an_invalid_lmw_whose_words_ahead_of_ra_are_held_takes_the_program_interrupt() {
    // The words from r26 on lie past the memory; the PPE never loads them.
    let (v, _) = run(LMW, 0x180, 0);
    assert_eq!(v, ExecuteVerdict::Fault(PpuFault::illegal("lmw")));
}

#[test]
fn an_invalid_load_string_whose_bytes_ahead_of_the_collision_run_into_a_hole_faults_on_the_hole() {
    // lswi r4, r5, 8: r5 = 0x114 is the second register, so 4 bytes come first.
    let lswi = PpuInstruction::Lswi {
        rt: 4,
        ra: 5,
        nb: 8,
    };
    let (v, _) = run(lswi, 0x116, 0);
    assert_eq!(
        v,
        ExecuteVerdict::MemFault(crate::exec::memory_helpers::unmapped(0x116))
    );
    let (v, _) = run(lswi, 0x118, 0);
    assert_eq!(v, ExecuteVerdict::Fault(PpuFault::illegal("lswi")));

    // lswx r4, r5, r6 with a count of 8: EA = 0x114 + 0x118, and r5 is
    // again the second register.
    let lswx = PpuInstruction::Lswx {
        rt: 4,
        ra: 5,
        rb: 6,
    };
    let (v, _) = run(lswx, 0x22E, 8);
    assert_eq!(
        v,
        ExecuteVerdict::MemFault(crate::exec::memory_helpers::unmapped(0x22E))
    );
    let (v, _) = run(lswx, 0x230, 8);
    assert_eq!(v, ExecuteVerdict::Fault(PpuFault::illegal("lswx")));
}

/// [CBE-Handbook p:290 s:9.10] An lmw alignment interrupt outranks a page-fault data storage interrupt.
#[test]
fn an_unaligned_invalid_lmw_with_words_ahead_of_ra_takes_the_alignment_interrupt() {
    // lmw r20, 1(r26): EA = 0x169, and the hole at 0x170 lies ahead of r26.
    let unaligned = PpuInstruction::Lmw {
        rt: 20,
        ra: 26,
        imm: 1,
    };
    let (v, effects) = run(unaligned, 0x170, 0);
    assert_eq!(v, ExecuteVerdict::Fault(PpuFault::alignment(0x169, false)));
    assert!(effects.is_empty(), "{effects:?}");
    // lmw r20, 1(r20) collides on its first register, so it loads nothing.
    let first = PpuInstruction::Lmw {
        rt: 20,
        ra: 20,
        imm: 1,
    };
    let (v, _) = run(first, 0x400, 0);
    assert_eq!(v, ExecuteVerdict::Fault(PpuFault::illegal("lmw")));
}

/// Runs `insn` as [`run`] does, over 0x400 bytes at 0 in one view that is
/// caching-inhibited.
fn run_inhibited(insn: PpuInstruction) -> ExecuteVerdict {
    let mut s = PpuState::new();
    for r in 1..32 {
        s.set_gpr(r, 0x100 + 4 * r as u64);
    }
    let mem = [0xAAu8; 0x400];
    let views = [cellgov_mem::RegionView {
        caching_inhibited: true,
        ..cellgov_mem::RegionView::plain(0, &mem)
    }];
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    execute(&insn, &mut s, uid(), &views, &mut effects, &mut store_buf)
}

/// [CBE-Handbook p:253 s:9.5.8] An lmw or load string operand in caching-inhibited storage takes the alignment interrupt.
#[test]
fn an_invalid_load_with_inhibited_bytes_ahead_of_the_collision_takes_the_alignment_interrupt() {
    assert_eq!(
        run_inhibited(LMW),
        ExecuteVerdict::Fault(PpuFault::alignment(0x168, false))
    );
    // lswi r4, r5, 8 loads r4 from r5 = 0x114 before the collision.
    let lswi = PpuInstruction::Lswi {
        rt: 4,
        ra: 5,
        nb: 8,
    };
    assert_eq!(
        run_inhibited(lswi),
        ExecuteVerdict::Fault(PpuFault::alignment(0x114, false))
    );
    // lswi r5, r5, 8 collides on its first register, so it loads nothing.
    let first = PpuInstruction::Lswi {
        rt: 5,
        ra: 5,
        nb: 8,
    };
    assert_eq!(
        run_inhibited(first),
        ExecuteVerdict::Fault(PpuFault::illegal("lswi"))
    );
}
