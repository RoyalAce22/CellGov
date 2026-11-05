//! The alignment interrupt for floating-point loads and stores, `lmw`
//! and `stmw`.

use super::*;

/// The first byte past the `0x400`-byte region the tests map at 0.
const UNMAPPED: u64 = 0x400;
/// The value r4 holds before the access, as the base of an update form.
const BASE: u64 = 0x100;
/// The value f1 holds before the access.
const F1: u64 = 0x4009_21FB_5444_2D18;

/// Runs `insn` with r4 = [`BASE`], r5 = `ea - BASE` and f1 = [`F1`] over
/// a mapped region of 0xAA bytes.
fn run(insn: PpuInstruction, ea: u64) -> (ExecuteVerdict, PpuState, Vec<Effect>) {
    let mut s = PpuState::new();
    s.set_gpr(4, BASE);
    s.set_gpr(5, ea.wrapping_sub(BASE));
    s.set_fpr(1, F1);
    let mut effects = Vec::new();
    let v = exec_with_mem(&insn, &mut s, 0, &[0xAAu8; 0x400], &mut effects);
    (v, s, effects)
}

/// Each floating-point form with EA = r4 + r5, and whether it stores.
fn floating_point() -> Vec<(PpuInstruction, bool)> {
    use PpuInstruction as I;
    let (frt, frs, ra, rb) = (1, 1, 4, 5);
    vec![
        (I::Lfsx { frt, ra, rb }, false),
        (I::Lfsux { frt, ra, rb }, false),
        (I::Lfdx { frt, ra, rb }, false),
        (I::Lfdux { frt, ra, rb }, false),
        (I::Stfsx { frs, ra, rb }, true),
        (I::Stfsux { frs, ra, rb }, true),
        (I::Stfdx { frs, ra, rb }, true),
        (I::Stfdux { frs, ra, rb }, true),
        (I::Stfiwx { frs, ra, rb }, true),
    ]
}

#[test]
fn a_misaligned_floating_point_access_faults_without_writing_a_register_or_memory() {
    for (insn, store) in floating_point() {
        for k in 1..=3 {
            let ea = BASE + 0x10 + k;
            let (v, s, effects) = run(insn, ea);
            assert_eq!(
                v,
                ExecuteVerdict::Fault(PpuFault::alignment(ea, store)),
                "{insn:?} at 0x{ea:x}"
            );
            assert_eq!(s.gpr[4], BASE, "{insn:?}: RA");
            assert_eq!(s.fpr[1], F1, "{insn:?}: f1");
            assert!(effects.is_empty(), "{insn:?}: {effects:?}");
        }
    }
}

#[test]
fn a_misaligned_floating_point_access_to_an_unmapped_ea_takes_the_alignment_fault() {
    for (insn, store) in floating_point() {
        let ea = UNMAPPED + 2;
        assert_eq!(
            run(insn, ea).0,
            ExecuteVerdict::Fault(PpuFault::alignment(ea, store)),
            "{insn:?}"
        );
    }
}

#[test]
fn a_word_aligned_floating_point_access_runs() {
    for (insn, _) in floating_point() {
        let ea = BASE + 0x14;
        let (v, s, _) = run(insn, ea);
        assert_eq!(v, ExecuteVerdict::Continue, "{insn:?} at 0x{ea:x}");
        if matches!(insn, PpuInstruction::Lfdx { .. }) {
            assert_eq!(s.fpr[1], 0xAAAA_AAAA_AAAA_AAAA);
        }
    }
}

#[test]
fn a_misaligned_lmw_or_stmw_faults_without_writing_a_register_or_memory() {
    for (insn, store) in [
        (
            PpuInstruction::Lmw {
                rt: 28,
                ra: 4,
                imm: 0x12,
            },
            false,
        ),
        (
            PpuInstruction::Stmw {
                rs: 28,
                ra: 4,
                imm: 0x12,
            },
            true,
        ),
    ] {
        let ea = BASE + 0x12;
        let (v, s, effects) = run(insn, ea);
        assert_eq!(
            v,
            ExecuteVerdict::Fault(PpuFault::alignment(ea, store)),
            "{insn:?}"
        );
        assert_eq!(s.gpr[28], 0, "{insn:?}: r28");
        assert!(effects.is_empty(), "{insn:?}: {effects:?}");
    }
}

#[test]
fn a_misaligned_lmw_or_stmw_to_an_unmapped_ea_takes_the_alignment_fault() {
    for (insn, store) in [
        (
            PpuInstruction::Lmw {
                rt: 28,
                ra: 4,
                imm: 0x301,
            },
            false,
        ),
        (
            PpuInstruction::Stmw {
                rs: 28,
                ra: 4,
                imm: 0x301,
            },
            true,
        ),
    ] {
        let ea = BASE + 0x301;
        assert!(ea >= UNMAPPED);
        assert_eq!(
            run(insn, ea).0,
            ExecuteVerdict::Fault(PpuFault::alignment(ea, store)),
            "{insn:?}"
        );
    }
}

#[test]
fn every_floating_point_load_and_store_and_stmw_admits_the_alignment_fault_class() {
    use crate::PpuFaultClass as C;
    let d_form = |op: u32| (op << 26) | (1 << 21) | (4 << 16);
    let x_form = |xo: u32| (31 << 26) | (1 << 21) | (4 << 16) | (5 << 11) | (xo << 1);
    // lfs lfsu lfd lfdu stfs stfsu stfd stfdu, stmw r28,0(r4).
    let mut words: Vec<u32> = (48..=55).map(d_form).collect();
    words.push((47 << 26) | (28 << 21) | (4 << 16));
    // lfsx lfsux lfdx lfdux stfsx stfsux stfdx stfdux stfiwx.
    words.extend([535, 567, 599, 631, 663, 695, 727, 759, 983].map(x_form));
    for raw in words {
        let descriptor = crate::decode::decode(raw).unwrap().fuzz_descriptor(raw);
        assert_eq!(
            descriptor.faults,
            &[C::Alignment, C::DataStorage],
            "0x{raw:08x}"
        );
    }
}
