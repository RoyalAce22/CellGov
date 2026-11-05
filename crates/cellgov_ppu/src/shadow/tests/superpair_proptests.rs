//! Generated-input properties of `make_super_pair`: a fused pair runs
//! and times the way its two halves run in sequence.

use proptest::prelude::*;

use super::make_super_pair;
use crate::exec::ExecuteVerdict;
use crate::instruction::{FusedPair, PpuInstruction};
use crate::shadow::semantics_support::{
    compare_immediate, cr_field, displacement, ds_displacement, reg, run_pair, run_sequence,
    state_seed, MEM_BASE,
};
use crate::state::PpuState;
use crate::timing::classify;

fn bc(bo: u8, bi: u8, offset: i16) -> PpuInstruction {
    PpuInstruction::Bc {
        bo,
        bi,
        offset: offset & !3,
        aa: false,
        link: false,
    }
}

/// A branch displacement, sometimes 4: a taken branch to the next
/// instruction lands where the untaken one does.
fn branch_offset() -> impl Strategy<Value = i16> {
    prop_oneof![
        3 => any::<i16>(),
        1 => Just(4i16),
    ]
}

/// A halfword, often one on either side of a sign boundary.
fn halfword() -> impl Strategy<Value = u16> {
    prop_oneof![
        1 => any::<u16>(),
        1 => prop::sample::select(vec![0u16, 0x7FFF, 0x8000, 0xFFFF]),
    ]
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
            branch_offset()
        )
            .prop_map(|(bf, ra, imm, bo, bi, off)| (
                PpuInstruction::Cmpwi { bf, ra, imm },
                bc(bo, bi, off)
            )),
        (cr_field(), reg(), reg(), reg(), branch_offset()).prop_map(|(bf, ra, bo, bi, off)| (
            PpuInstruction::CmpwZero { bf, ra },
            bc(bo, bi, off),
        )),
        (cr_field(), reg(), reg(), reg(), reg(), branch_offset()).prop_map(
            |(bf, ra, rb, bo, bi, off)| (PpuInstruction::Cmpw { bf, ra, rb }, bc(bo, bi, off))
        ),
        (
            cr_field(),
            reg(),
            compare_immediate(),
            reg(),
            reg(),
            branch_offset()
        )
            .prop_map(|(bf, ra, imm, bo, bi, off)| (
                PpuInstruction::Cmpdi { bf, ra, imm },
                bc(bo, bi, off)
            )),
        (cr_field(), reg(), reg(), reg(), branch_offset()).prop_map(|(bf, ra, bo, bi, off)| (
            PpuInstruction::CmpdZero { bf, ra },
            bc(bo, bi, off),
        )),
        (cr_field(), reg(), reg(), reg(), reg(), branch_offset()).prop_map(
            |(bf, ra, rb, bo, bi, off)| (PpuInstruction::Cmpd { bf, ra, rb }, bc(bo, bi, off))
        ),
        (cr_field(), reg(), reg(), reg(), reg(), branch_offset()).prop_map(
            |(bf, ra, rb, bo, bi, off)| (PpuInstruction::Cmplw { bf, ra, rb }, bc(bo, bi, off))
        ),
        // The shadow quickens `ori 0, 0, 0` to Nop before pairing, so
        // this arm moves lo = 0 on r0 to 1.
        (reg(), halfword(), halfword()).prop_map(|(rt, hi, lo)| (
            PpuInstruction::Lis { rt, imm: hi as i16 },
            PpuInstruction::Ori {
                ra: rt,
                rs: rt,
                imm: if rt == 0 && lo == 0 { 1 } else { lo },
            },
        )),
        // A first load into a nonzero base moves the second EA and does not
        // fuse, so this arm moves rT1 off it.
        (reg(), reg(), reg(), ds_displacement()).prop_map(|(rt1, rt2, ra, off)| {
            let rt1 = if rt1 == ra && ra != 0 {
                (ra + 1) & 31
            } else {
                rt1
            };
            let off1 = off.min(i16::MAX - 11);
            (
                PpuInstruction::Ld {
                    rt: rt1,
                    ra,
                    imm: off1,
                },
                PpuInstruction::Ld {
                    rt: rt2,
                    ra,
                    imm: off1 + 8,
                },
            )
        }),
        // The shadow quickens `or rA, rS, rS` to `mr` before pairing, so
        // this arm gives that shape as `Mr`.
        (reg(), reg(), reg(), reg(), reg(), reg()).prop_map(|(ra, rs, rb, rt, add_ra, add_rb)| {
            let or = if rs == rb {
                PpuInstruction::Mr { ra, rs }
            } else {
                PpuInstruction::Or {
                    ra,
                    rs,
                    rb,
                    rc: false,
                }
            };
            (
                or,
                PpuInstruction::Add {
                    rt,
                    ra: add_ra,
                    rb: add_rb,
                    oe: false,
                    rc: false,
                },
            )
        }),
        // An lbzu with RA = 0 or RA = RT is invalid and does not fuse, so
        // this arm moves RA off both. The compare reads RT, RA or another
        // register.
        (reg(), reg(), displacement(), cr_field(), 0u8..3, reg()).prop_map(
            |(rt, ra, imm, bf, pick, other)| {
                let ra = if ra == 0 || ra == rt { rt % 31 + 1 } else { ra };
                let cmp_ra = [rt, ra, other][usize::from(pick)];
                (
                    PpuInstruction::Lbzu { rt, ra, imm },
                    PpuInstruction::CmpwZero { bf, ra: cmp_ra },
                )
            }
        ),
        // An ldu with RA = 0 or RA = RT is invalid, and a second base equal
        // to RT1 does not fuse, so this arm moves each base off those. Half
        // the cases share one base.
        (
            reg(),
            reg(),
            ds_displacement(),
            reg(),
            reg(),
            ds_displacement(),
            any::<bool>()
        )
            .prop_map(|(rt1, ra1, off1, rt2, ra2, off2, same)| {
                let ra1 = if ra1 == 0 || ra1 == rt1 {
                    rt1 % 31 + 1
                } else {
                    ra1
                };
                let mut ra2 = if same { ra1 } else { ra2 };
                // The guard rejects three values, so four steps always
                // reach a valid base.
                for _ in 0..4 {
                    if ra2 != 0 && ra2 != rt2 && ra2 != rt1 {
                        break;
                    }
                    ra2 = ra2 % 31 + 1;
                }
                (
                    PpuInstruction::Ldu {
                        rt: rt1,
                        ra: ra1,
                        imm: off1,
                    },
                    PpuInstruction::Ldu {
                        rt: rt2,
                        ra: ra2,
                        imm: off2,
                    },
                )
            }),
        (cr_field(), reg(), reg(), reg(), reg(), reg()).prop_map(
            |(bf, ra, rb, nor_ra, nor_rs, nor_rb)| (
                PpuInstruction::Cmpd { bf, ra, rb },
                PpuInstruction::Nor {
                    ra: nor_ra,
                    rs: nor_rs,
                    rb: nor_rb,
                    rc: false,
                },
            )
        ),
        // The shadow quickens a bclr to Blr only when BO is 1z1zz.
        (
            reg(),
            reg(),
            prop::sample::select(vec![20u8, 21, 22, 23, 28, 29, 30, 31]),
            reg()
        )
            .prop_map(|(ra, rs, bo, bi)| (
                PpuInstruction::Extsw { ra, rs, rc: false },
                PpuInstruction::Blr { bo, bi },
            )),
        // The shadow quickens `addi rT, 0, imm` to `li` before pairing, so
        // this arm gives that shape as `Li`.
        (reg(), reg(), any::<i16>(), reg(), reg(), branch_offset()).prop_map(
            |(rt, ra, imm, bo, bi, off)| {
                let addi = if ra == 0 {
                    PpuInstruction::Li { rt, imm }
                } else {
                    PpuInstruction::Addi { rt, ra, imm }
                };
                (addi, bc(bo, bi, off))
            }
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
/// compare is the second half of `lwz; cmpwi`. It is the first half of
/// `cmpd; nor` and of each compare-and-`bc` pair. Thus either slot may
/// hold the changed instruction.
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
        (PpuInstruction::Cmpdi { bf, ra, .. }, PpuInstruction::Bc { .. }) => {
            let seen = i16::try_from(state.gpr[ra as usize] as i64).ok();
            match seen.map(|v| v.wrapping_add(nudge)) {
                Some(imm) => (PpuInstruction::Cmpdi { bf, ra, imm }, b),
                None => (a, b),
            }
        }
        (
            PpuInstruction::Cmpd { ra, rb, .. },
            PpuInstruction::Bc { .. } | PpuInstruction::Nor { .. },
        ) => {
            let seen = state.gpr[ra as usize] as i64;
            state.set_gpr(rb as usize, seen.wrapping_add(i64::from(nudge)) as u64);
            (a, b)
        }
        // The low word moves by `nudge` and the high word flips, so a
        // compare that reads all 64 bits disagrees with cmplw.
        (PpuInstruction::Cmplw { ra, rb, .. }, PpuInstruction::Bc { .. }) => {
            let seen = state.gpr[ra as usize];
            let low = (seen as u32).wrapping_add(nudge as i32 as u32);
            state.set_gpr(
                rb as usize,
                (!seen & 0xFFFF_FFFF_0000_0000) | u64::from(low),
            );
            (a, b)
        }
        _ => (a, b),
    }
}

proptest! {
    // Seven of the twenty-three arms are compares that need an aimed
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
        let refused = !matches!(verdict_pair, ExecuteVerdict::Continue | ExecuteVerdict::Branch);
        if refused && matches!(fused, PpuInstruction::Fused(FusedPair::LduLdu { .. })) {
            // The ldu pair's first half writes the base it reads, so a
            // refused pair leaves the state it began in. The batch's
            // split rerun reaches the halves' state from there.
            prop_assert_eq!(pair.fingerprint(), seed.state().fingerprint());
        } else {
            prop_assert_eq!(pair.fingerprint(), halves.fingerprint());
        }
        prop_assert_eq!(effects_pair, effects_halves);
        // The executor reports a refusal inside the second half at the
        // pair's address. The batch then runs the pair split, so the
        // unit stops at the second slot as the halves do
        // (`unit/tests/fused_refusal_tests.rs`).
        if matches!(verdict_pair, ExecuteVerdict::Continue | ExecuteVerdict::Branch) {
            prop_assert_eq!(pair.pc, halves.pc);
        }
    }

    #[test]
    fn a_fused_pair_times_like_its_halves(
        (a, b) in fusable_pair(),
        seed in state_seed(),
    ) {
        let Some(fused) = make_super_pair(a, b) else {
            return Err(TestCaseError::fail(format!("{a:?} + {b:?} did not fuse")));
        };
        let mem = seed.memory();
        let mut state = seed.state();
        let ([first, second], n) = classify(&fused, &state);
        prop_assert_eq!(n, 2);
        let ([raw_first, _], _) = classify(&a, &state);
        prop_assert_eq!(first, raw_first);
        let (verdict, _) = run_sequence(&[a], &mut state, &mem);
        if verdict != ExecuteVerdict::Continue {
            return Ok(());
        }
        let ([raw_second, _], _) = classify(&b, &state);
        prop_assert_eq!(second, raw_second);
    }
}
