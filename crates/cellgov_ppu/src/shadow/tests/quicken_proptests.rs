//! Generated-input properties of `quicken_insn`: every rewrite leaves
//! the machine where the original instruction does.

use proptest::prelude::*;

use super::quicken_insn;
use crate::instruction::PpuInstruction;
use crate::shadow::semantics_support::{reg, run_sequence, state_seed, StateSeed};

/// Instructions every quickening rule accepts, one arm per rule.
fn quickenable() -> impl Strategy<Value = PpuInstruction> {
    prop_oneof![
        (reg(), any::<i16>()).prop_map(|(rt, imm)| PpuInstruction::Addi { rt, ra: 0, imm }),
        (reg(), 1u8..32).prop_map(|(ra, step)| PpuInstruction::Or {
            ra,
            rs: (ra + step) % 32,
            rb: (ra + step) % 32,
            rc: false,
        }),
        (reg(), reg(), 1u8..32).prop_map(|(ra, rs, sh)| PpuInstruction::Rlwinm {
            ra,
            rs,
            sh,
            mb: 0,
            me: 31 - sh,
            rc: false,
        }),
        (reg(), reg(), 1u8..32).prop_map(|(ra, rs, n)| PpuInstruction::Rlwinm {
            ra,
            rs,
            sh: 32 - n,
            mb: n,
            me: 31,
            rc: false,
        }),
        (reg(), reg(), 0u8..32).prop_map(|(ra, rs, n)| PpuInstruction::Rlwinm {
            ra,
            rs,
            sh: 0,
            mb: n,
            me: 31,
            rc: false,
        }),
        reg().prop_map(|ra| PpuInstruction::Ori { ra, rs: ra, imm: 0 }),
        (0u8..8, reg()).prop_map(|(bf, ra)| PpuInstruction::Cmpwi { bf, ra, imm: 0 }),
        (reg(), reg(), 0u8..64).prop_map(|(ra, rs, n)| PpuInstruction::Rldicl {
            ra,
            rs,
            sh: 0,
            mb: n,
            rc: false,
        }),
        (reg(), reg(), 1u8..64).prop_map(|(ra, rs, n)| PpuInstruction::Rldicr {
            ra,
            rs,
            sh: n,
            me: 63 - n,
            rc: false,
        }),
        (reg(), reg(), 1u8..64).prop_map(|(ra, rs, n)| PpuInstruction::Rldicl {
            ra,
            rs,
            sh: 64 - n,
            mb: n,
            rc: false,
        }),
    ]
}

/// Verdict, registers, PC and emitted effects after `insn` runs from
/// the seed's state.
fn outcome(insn: PpuInstruction, seed: &StateSeed, mem: &[u8]) -> String {
    let mut state = seed.state();
    let (verdict, effects) = run_sequence(&[insn], &mut state, mem);
    format!(
        "{verdict:?} pc={:#x} {:?} {effects:?}",
        state.pc,
        state.fingerprint()
    )
}

proptest! {
    #[test]
    fn a_quickened_instruction_leaves_the_machine_where_the_original_does(
        insn in quickenable(),
        seed in state_seed(),
    ) {
        let Some(quick) = quicken_insn(insn) else {
            return Err(TestCaseError::fail(format!("{insn:?} did not quicken")));
        };
        prop_assert_ne!(quick, insn);
        let mem = seed.memory();
        prop_assert_eq!(outcome(insn, &seed, &mem), outcome(quick, &seed, &mem));
    }
}

#[test]
fn or_n_n_n_is_never_rewritten_for_any_register() {
    for n in 0..32u8 {
        let hint = PpuInstruction::Or {
            ra: n,
            rs: n,
            rb: n,
            rc: false,
        };
        assert_eq!(quicken_insn(hint), None, "or {n},{n},{n}");
    }
}
