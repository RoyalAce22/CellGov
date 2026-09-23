//! Generated-input properties of `make_super_pair`: a fused pair runs
//! the way its two halves run in sequence.

use proptest::prelude::*;

use super::make_super_pair;
use crate::exec::ExecuteVerdict;
use crate::instruction::PpuInstruction;
use crate::shadow::semantics_support::{
    compare_immediate, cr_field, displacement, ds_displacement, reg, run_pair, run_sequence,
    state_seed, MEM_BASE,
};
use crate::state::PpuState;

fn bc(bo: u8, bi: u8, offset: i16) -> PpuInstruction {
    PpuInstruction::Bc {
        bo,
        bi,
        offset: offset & !3,
        aa: false,
        link: false,
    }
}

/// Instruction pairs every fusion rule accepts, one arm per rule.
fn fusable_pair() -> impl Strategy<Value = (PpuInstruction, PpuInstruction)> {
    prop_oneof![
        (
            reg(),
            reg(),
            displacement(),
            cr_field(),
            compare_immediate()
        )
            .prop_map(|(rt, ra, imm, bf, cmp_imm)| (
                PpuInstruction::Lwz { rt, ra, imm },
                PpuInstruction::Cmpwi {
                    bf,
                    ra: rt,
                    imm: cmp_imm
                },
            )),
        (reg(), any::<i16>(), reg(), displacement()).prop_map(|(rt, imm, ra, off)| (
            PpuInstruction::Li { rt, imm },
            PpuInstruction::Stw {
                rs: rt,
                ra,
                imm: off
            },
        )),
        (reg(), reg(), displacement()).prop_map(|(rt, ra, imm)| (
            PpuInstruction::Mflr { rt },
            PpuInstruction::Stw { rs: rt, ra, imm },
        )),
        (reg(), reg(), displacement()).prop_map(|(rt, ra, imm)| (
            PpuInstruction::Lwz { rt, ra, imm },
            PpuInstruction::Mtlr { rs: rt },
        )),
        (reg(), reg(), ds_displacement()).prop_map(|(rt, ra, imm)| (
            PpuInstruction::Mflr { rt },
            PpuInstruction::Std { rs: rt, ra, imm },
        )),
        (reg(), reg(), ds_displacement()).prop_map(|(rt, ra, imm)| (
            PpuInstruction::Ld { rt, ra, imm },
            PpuInstruction::Mtlr { rs: rt },
        )),
        (reg(), reg(), reg(), ds_displacement()).prop_map(|(rs1, rs2, ra, off)| {
            let off1 = off.min(i16::MAX - 8);
            (
                PpuInstruction::Std {
                    rs: rs1,
                    ra,
                    imm: off1,
                },
                PpuInstruction::Std {
                    rs: rs2,
                    ra,
                    imm: off1 + 8,
                },
            )
        }),
        (reg(), reg(), displacement(), cr_field()).prop_map(|(rt, ra, imm, bf)| (
            PpuInstruction::Lwz { rt, ra, imm },
            PpuInstruction::CmpwZero { bf, ra: rt },
        )),
        (
            cr_field(),
            reg(),
            compare_immediate(),
            reg(),
            reg(),
            any::<i16>()
        )
            .prop_map(|(bf, ra, imm, bo, bi, off)| (
                PpuInstruction::Cmpwi { bf, ra, imm },
                bc(bo, bi, off)
            )),
        (cr_field(), reg(), reg(), reg(), any::<i16>()).prop_map(|(bf, ra, bo, bi, off)| (
            PpuInstruction::CmpwZero { bf, ra },
            bc(bo, bi, off),
        )),
        (cr_field(), reg(), reg(), reg(), reg(), any::<i16>()).prop_map(
            |(bf, ra, rb, bo, bi, off)| (PpuInstruction::Cmpw { bf, ra, rb }, bc(bo, bi, off))
        ),
    ]
}

/// The word at `ea` in the region, if the region holds all of it.
fn word_at(mem: &[u8], ea: u64) -> Option<i32> {
    let off = usize::try_from(ea.checked_sub(MEM_BASE)?).ok()?;
    let bytes = mem.get(off..off.checked_add(4)?)?;
    Some(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Aim a compare at the value it will see, plus `nudge`, so an
/// off-by-one in the fused compare changes the condition register. A
/// random immediate meets a random operand too rarely for that. The
/// compare is the second half of `lwz; cmpwi` and the first half of
/// `cmpwi; bc` and `cmpw; bc`, so either slot may hold the changed
/// instruction.
fn aim_compare(
    a: PpuInstruction,
    b: PpuInstruction,
    state: &mut PpuState,
    mem: &[u8],
    nudge: i16,
) -> (PpuInstruction, PpuInstruction) {
    let near = |seen: i32| i16::try_from(seen).ok().map(|v| v.wrapping_add(nudge));
    match (a, b) {
        (PpuInstruction::Lwz { ra, imm, .. }, PpuInstruction::Cmpwi { bf, ra: cmp_ra, .. }) => {
            match word_at(mem, state.ea_d_form(ra, imm)).and_then(near) {
                Some(imm) => (
                    a,
                    PpuInstruction::Cmpwi {
                        bf,
                        ra: cmp_ra,
                        imm,
                    },
                ),
                None => (a, b),
            }
        }
        (PpuInstruction::Cmpwi { bf, ra, .. }, PpuInstruction::Bc { .. }) => {
            match near(state.gpr[ra as usize] as i32) {
                Some(imm) => (PpuInstruction::Cmpwi { bf, ra, imm }, b),
                None => (a, b),
            }
        }
        (PpuInstruction::Cmpw { ra, rb, .. }, PpuInstruction::Bc { .. }) => {
            let seen = state.gpr[ra as usize] as i32;
            state.set_gpr(
                rb as usize,
                seen.wrapping_add(i32::from(nudge)) as i64 as u64,
            );
            (a, b)
        }
        _ => (a, b),
    }
}

proptest! {
    // The compare arms are three of eleven and need an aimed
    // operand to show an off-by-one. Four times the default count
    // makes one such case near certain per run.
    #![proptest_config(ProptestConfig::with_cases(1024))]

    #[test]
    fn a_fused_pair_leaves_the_machine_where_its_halves_do(
        (a, b) in fusable_pair(),
        seed in state_seed(),
        aim in proptest::option::weighted(0.8, -1i16..=1),
    ) {
        let mem = seed.memory();
        let mut halves = seed.state();
        let mut pair = seed.state();
        let (a, b) = match aim {
            Some(nudge) => {
                let aimed = aim_compare(a, b, &mut halves, &mem, nudge);
                prop_assert_eq!(aim_compare(a, b, &mut pair, &mem, nudge), aimed);
                aimed
            }
            None => (a, b),
        };
        let Some(fused) = make_super_pair(a, b) else {
            return Err(TestCaseError::fail(format!("{a:?} + {b:?} did not fuse")));
        };
        prop_assert!(fused.is_super_pair());

        let (verdict_halves, effects_halves) = run_sequence(&[a, b], &mut halves, &mem);
        let (verdict_pair, effects_pair) = run_pair(&fused, &mut pair, &mem);

        prop_assert_eq!(&verdict_pair, &verdict_halves);
        prop_assert_eq!(pair.fingerprint(), halves.fingerprint());
        prop_assert_eq!(effects_pair, effects_halves);
        // The pair reports a fault inside its second half at its own
        // address; the halves report it at the second slot.
        if matches!(verdict_pair, ExecuteVerdict::Continue | ExecuteVerdict::Branch) {
            prop_assert_eq!(pair.pc, halves.pc);
        }
    }
}
