use super::{classify, PpeTiming};
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;

/// The issue cycle of each op of `insns`, retired once as straight-line
/// code from a fresh model.
fn issued(insns: &[I]) -> Vec<u64> {
    let state = PpuState::new();
    let mut timing = PpeTiming::new();
    let mut cycles = Vec::new();
    for (i, insn) in insns.iter().enumerate() {
        let at = 0x1000 + 4 * i as u64;
        let (ops, n) = classify(insn, &state);
        let issued = timing.retire_instruction(at, &ops[..n], at + 4 * n.max(1) as u64);
        cycles.extend_from_slice(&issued[..n]);
    }
    cycles
}

const MFVSCR: I = I::Mfvscr { vt: 3 };
const LI: I = I::Addi {
    rt: 10,
    ra: 0,
    imm: 1,
};
const LWZ_FROM_R3: I = I::Lwz {
    rt: 4,
    ra: 3,
    imm: 0,
};

/// The cycles from `update` to an `lwz` that reads its updated base,
/// after `before`.
fn base_wait(before: I, update: I) -> u64 {
    let cycles = issued(&[before, update, LWZ_FROM_R3]);
    cycles[2] - cycles[1]
}

#[test]
fn an_update_form_fp_load_or_store_after_an_mfvscr_writes_its_base_as_it_does_after_an_li() {
    let forms = [
        I::Stfdu {
            frs: 1,
            ra: 3,
            imm: 8,
        },
        I::Lfdu {
            frt: 1,
            ra: 3,
            imm: 8,
        },
    ];
    for update in forms {
        assert_eq!(
            base_wait(MFVSCR, update),
            base_wait(LI, update),
            "{update:?}: the lwz waits longer for the base after an mfvscr",
        );
    }
}
