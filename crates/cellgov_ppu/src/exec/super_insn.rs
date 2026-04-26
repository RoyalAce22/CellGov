//! Predecoded shadow output: quickened single-instruction rewrites
//! and super-paired 2-instruction fusions. Profiling-driven; the
//! shadow build picks candidates above frequency thresholds and
//! rewrites them into these specialized variants. None of these
//! arms is ISA-native; they all decompose into one or two real PPC
//! instructions whose execution semantics they replicate.

use crate::exec::branch::branch_condition;
use crate::exec::{buffer_store, load_ze, ExecuteVerdict};
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

pub(crate) fn execute(
    insn: &PpuInstruction,
    state: &mut PpuState,
    region_views: &[(u64, &[u8])],
    store_buf: &mut StoreBuffer,
) -> ExecuteVerdict {
    match *insn {
        // Quickened (specialized) forms
        PpuInstruction::Li { rt, imm } => {
            state.gpr[rt as usize] = imm as i64 as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Mr { ra, rs } => {
            state.gpr[ra as usize] = state.gpr[rs as usize];
            ExecuteVerdict::Continue
        }
        PpuInstruction::Slwi { ra, rs, n } => {
            let val = (state.gpr[rs as usize] as u32) << n;
            state.gpr[ra as usize] = val as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Srwi { ra, rs, n } => {
            let val = (state.gpr[rs as usize] as u32) >> n;
            state.gpr[ra as usize] = val as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Clrlwi { ra, rs, n } => {
            let mask = if n >= 32 { 0 } else { u32::MAX >> n };
            let val = (state.gpr[rs as usize] as u32) & mask;
            state.gpr[ra as usize] = val as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Nop => ExecuteVerdict::Continue,
        PpuInstruction::CmpwZero { bf, ra } => {
            let a = state.gpr[ra as usize] as i32;
            let cr_val = if a < 0 {
                0b1000
            } else if a > 0 {
                0b0100
            } else {
                0b0010
            };
            // Book I 3.3.9: CR[4*BF .. 4*BF+3] <- c || XER[SO]. The
            // SO bit is sticky and must be reflected in every
            // compare result.
            state.set_cr_field(bf, cr_val | u8::from(state.xer_so()));
            ExecuteVerdict::Continue
        }
        PpuInstruction::Clrldi { ra, rs, n } => {
            let mask = if n >= 64 { 0 } else { u64::MAX >> n };
            state.gpr[ra as usize] = state.gpr[rs as usize] & mask;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Sldi { ra, rs, n } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] << n;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Srdi { ra, rs, n } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] >> n;
            ExecuteVerdict::Continue
        }

        // Superinstructions (compound 2-instruction pairs)
        PpuInstruction::LwzCmpwi {
            rt,
            ra_load,
            offset,
            bf,
            cmp_imm,
        } => {
            let ea = state.ea_d_form(ra_load, offset);
            match load_ze(region_views, store_buf, ea, 4) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    let a = val as i32;
                    let b = cmp_imm as i32;
                    let cr_val = if a < b {
                        0b1000
                    } else if a > b {
                        0b0100
                    } else {
                        0b0010
                    };
                    state.set_cr_field(bf, cr_val | u8::from(state.xer_so()));
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::LiStw {
            rt,
            imm,
            ra_store,
            store_offset,
        } => {
            let val = imm as i64 as u64;
            state.gpr[rt as usize] = val;
            let ea = state.ea_d_form(ra_store, store_offset);
            buffer_store(store_buf, state, ea, 4, val)
        }
        PpuInstruction::MflrStw {
            rt,
            ra_store,
            store_offset,
        } => {
            state.gpr[rt as usize] = state.lr;
            let ea = state.ea_d_form(ra_store, store_offset);
            buffer_store(store_buf, state, ea, 4, state.gpr[rt as usize])
        }
        PpuInstruction::LwzMtlr {
            rt,
            ra_load,
            offset,
        } => {
            let ea = state.ea_d_form(ra_load, offset);
            match load_ze(region_views, store_buf, ea, 4) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    state.lr = val;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::MflrStd {
            rt,
            ra_store,
            store_offset,
        } => {
            state.gpr[rt as usize] = state.lr;
            let ea = state.ea_d_form(ra_store, store_offset);
            buffer_store(store_buf, state, ea, 8, state.gpr[rt as usize])
        }
        PpuInstruction::LdMtlr {
            rt,
            ra_load,
            offset,
        } => {
            let ea = state.ea_d_form(ra_load, offset);
            match load_ze(region_views, store_buf, ea, 8) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    state.lr = val;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::StdStd {
            rs1,
            rs2,
            ra,
            offset1,
        } => {
            // The shadow pairing pass only emits this variant when
            // `off2 == off1 + 8` and the two stores share RA (see
            // shadow::fuse_pair). EA2 = EA1 + 8 is correct by
            // construction; if that constraint is ever relaxed,
            // StdStd needs an `offset2` field.
            let ea1 = state.ea_d_form(ra, offset1);
            let v1 = buffer_store(store_buf, state, ea1, 8, state.gpr[rs1 as usize]);
            if !matches!(v1, ExecuteVerdict::Continue) {
                return v1;
            }
            let ea2 = ea1.wrapping_add(8);
            buffer_store(store_buf, state, ea2, 8, state.gpr[rs2 as usize])
        }
        PpuInstruction::CmpwiBc {
            bf,
            ra,
            imm,
            bo,
            bi,
            target_offset,
        } => {
            let a = state.gpr[ra as usize] as i32;
            let b = imm as i32;
            let cr_val = if a < b {
                0b1000
            } else if a > b {
                0b0100
            } else {
                0b0010
            };
            state.set_cr_field(bf, cr_val | u8::from(state.xer_so()));
            // target_offset is relative to the bc slot (super + 4).
            if branch_condition(state, bo, bi) {
                state.pc =
                    (state.pc.wrapping_add(4) as i64).wrapping_add(target_offset as i64) as u64;
                ExecuteVerdict::Branch
            } else {
                ExecuteVerdict::Continue
            }
        }
        PpuInstruction::CmpwBc {
            bf,
            ra,
            rb,
            bo,
            bi,
            target_offset,
        } => {
            let a = state.gpr[ra as usize] as i32;
            let b = state.gpr[rb as usize] as i32;
            let cr_val = if a < b {
                0b1000
            } else if a > b {
                0b0100
            } else {
                0b0010
            };
            state.set_cr_field(bf, cr_val | u8::from(state.xer_so()));
            // target_offset is relative to the bc slot (super + 4).
            if branch_condition(state, bo, bi) {
                state.pc =
                    (state.pc.wrapping_add(4) as i64).wrapping_add(target_offset as i64) as u64;
                ExecuteVerdict::Branch
            } else {
                ExecuteVerdict::Continue
            }
        }
        PpuInstruction::Consumed => {
            unreachable!("Consumed slots should be skipped by the fetch loop")
        }

        _ => unreachable!("super_insn::execute called with non-super variant"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::test_support::{exec_no_mem, exec_with_mem};
    use cellgov_effects::Effect;

    #[test]
    fn li_negative_sign_extends() {
        let mut s = PpuState::new();
        exec_no_mem(&PpuInstruction::Li { rt: 5, imm: -1 }, &mut s);
        assert_eq!(s.gpr[5], u64::MAX);
    }

    #[test]
    fn slwi_shifts_left() {
        let mut s = PpuState::new();
        s.gpr[4] = 0x0000_0001;
        exec_no_mem(&PpuInstruction::Slwi { ra: 3, rs: 4, n: 8 }, &mut s);
        assert_eq!(s.gpr[3], 0x100);
    }

    #[test]
    fn srwi_shifts_right() {
        let mut s = PpuState::new();
        s.gpr[4] = 0x0000_FF00;
        exec_no_mem(&PpuInstruction::Srwi { ra: 3, rs: 4, n: 8 }, &mut s);
        assert_eq!(s.gpr[3], 0xFF);
    }

    #[test]
    fn clrlwi_clears_high_bits() {
        let mut s = PpuState::new();
        s.gpr[4] = 0xFFFF_FFFF;
        exec_no_mem(
            &PpuInstruction::Clrlwi {
                ra: 3,
                rs: 4,
                n: 16,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 0x0000_FFFF);
    }

    #[test]
    fn clrlwi_n32_zeroes_all() {
        let mut s = PpuState::new();
        s.gpr[4] = 0xFFFF_FFFF;
        exec_no_mem(
            &PpuInstruction::Clrlwi {
                ra: 3,
                rs: 4,
                n: 32,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 0);
    }

    #[test]
    fn nop_returns_continue() {
        let mut s = PpuState::new();
        let result = exec_no_mem(&PpuInstruction::Nop, &mut s);
        assert_eq!(result, ExecuteVerdict::Continue);
    }

    #[test]
    fn cmpw_zero_negative() {
        let mut s = PpuState::new();
        s.gpr[3] = (-5i64) as u64;
        exec_no_mem(&PpuInstruction::CmpwZero { bf: 0, ra: 3 }, &mut s);
        assert_eq!(s.cr_field(0), 0b1000); // LT
    }

    #[test]
    fn cmpw_zero_equal() {
        let mut s = PpuState::new();
        s.gpr[3] = 0;
        exec_no_mem(&PpuInstruction::CmpwZero { bf: 0, ra: 3 }, &mut s);
        assert_eq!(s.cr_field(0), 0b0010); // EQ
    }

    #[test]
    fn clrldi_zero_mask_clears_all() {
        let mut s = PpuState::new();
        s.gpr[4] = 0xDEAD_BEEF_CAFE_BABE;
        exec_no_mem(
            &PpuInstruction::Clrldi {
                ra: 3,
                rs: 4,
                n: 64,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 0);
    }

    #[test]
    fn sldi_large_shift() {
        let mut s = PpuState::new();
        s.gpr[4] = 1;
        exec_no_mem(
            &PpuInstruction::Sldi {
                ra: 3,
                rs: 4,
                n: 63,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 0x8000_0000_0000_0000);
    }

    #[test]
    fn srdi_large_shift() {
        let mut s = PpuState::new();
        s.gpr[4] = 0x8000_0000_0000_0000;
        exec_no_mem(
            &PpuInstruction::Srdi {
                ra: 3,
                rs: 4,
                n: 63,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 1);
    }

    #[test]
    fn lwz_cmpwi_lt_and_gt() {
        let mut mem = vec![0u8; 0x2000];
        mem[0x100..0x104].copy_from_slice(&5u32.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::LwzCmpwi {
                rt: 3,
                ra_load: 1,
                offset: 0,
                bf: 2,
                cmp_imm: 10,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3], 5);
        assert_eq!(s.cr_field(2), 0b1000); // LT

        mem[0x100..0x104].copy_from_slice(&20u32.to_be_bytes());
        let mut s2 = PpuState::new();
        s2.gpr[1] = 0x100;
        let mut effects2 = Vec::new();
        exec_with_mem(
            &PpuInstruction::LwzCmpwi {
                rt: 3,
                ra_load: 1,
                offset: 0,
                bf: 2,
                cmp_imm: 10,
            },
            &mut s2,
            0,
            &mem,
            &mut effects2,
        );
        assert_eq!(s2.gpr[3], 20);
        assert_eq!(s2.cr_field(2), 0b0100); // GT
    }

    #[test]
    fn lwz_cmpwi_mem_fault() {
        let mut s = PpuState::new();
        s.gpr[1] = 0xFFFF_FFFF;
        let result = exec_no_mem(
            &PpuInstruction::LwzCmpwi {
                rt: 3,
                ra_load: 1,
                offset: 0,
                bf: 0,
                cmp_imm: 0,
            },
            &mut s,
        );
        assert!(matches!(result, ExecuteVerdict::MemFault(_)));
    }

    #[test]
    fn li_stw_negative_imm() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::LiStw {
                rt: 3,
                imm: -1,
                ra_store: 1,
                store_offset: 0,
            },
            &mut s,
            0,
            &[0u8; 0x2000],
            &mut effects,
        );
        assert_eq!(s.gpr[3], u64::MAX);
    }

    #[test]
    fn lwz_mtlr_mem_fault() {
        let mut s = PpuState::new();
        s.gpr[1] = 0xFFFF_FFFF;
        let result = exec_no_mem(
            &PpuInstruction::LwzMtlr {
                rt: 0,
                ra_load: 1,
                offset: 0,
            },
            &mut s,
        );
        assert!(matches!(result, ExecuteVerdict::MemFault(_)));
    }

    #[test]
    fn cmpwi_bc_not_taken() {
        let mut s = PpuState::new();
        s.pc = 0x1000;
        s.gpr[3] = 5;
        let v = exec_no_mem(
            &PpuInstruction::CmpwiBc {
                bf: 0,
                ra: 3,
                imm: 10,
                bo: 0x0C,
                bi: 2,
                target_offset: 16,
            },
            &mut s,
        );
        assert_eq!(v, ExecuteVerdict::Continue);
        assert_eq!(s.cr_field(0), 0b1000); // LT
        assert_eq!(s.pc, 0x1000);
    }

    #[test]
    fn cmpwi_bc_gt_taken() {
        let mut s = PpuState::new();
        s.pc = 0x2000;
        s.gpr[3] = 10;
        let v = exec_no_mem(
            &PpuInstruction::CmpwiBc {
                bf: 0,
                ra: 3,
                imm: 5,
                bo: 0x0C,
                bi: 1, // GT bit of cr0
                target_offset: 20,
            },
            &mut s,
        );
        assert_eq!(v, ExecuteVerdict::Branch);
        assert_eq!(s.cr_field(0), 0b0100); // GT
        assert_eq!(s.pc, 0x2018);
    }

    #[test]
    fn cmpw_bc_not_taken() {
        let mut s = PpuState::new();
        s.pc = 0x1000;
        s.gpr[3] = 5;
        s.gpr[4] = 10;
        let v = exec_no_mem(
            &PpuInstruction::CmpwBc {
                bf: 0,
                ra: 3,
                rb: 4,
                bo: 0x0C,
                bi: 2,
                target_offset: 16,
            },
            &mut s,
        );
        assert_eq!(v, ExecuteVerdict::Continue);
        assert_eq!(s.cr_field(0), 0b1000); // LT
    }

    // li_stw needs at least one effect to be emitted; verify that
    // separately from li_stw_negative_imm so the existence test is
    // independent.
    #[test]
    fn li_stw_emits_store_effect() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::LiStw {
                rt: 5,
                imm: 99,
                ra_store: 1,
                store_offset: 0,
            },
            &mut s,
            0,
            &[0u8; 0x2000],
            &mut effects,
        );
        assert_eq!(s.gpr[5], 99);
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::SharedWriteIntent { .. })));
    }

    #[test]
    fn li_stw_stores_low_32_bits_only() {
        // LiStw with imm=-1 sign-extends to 0xFFFF_FFFF_FFFF_FFFF in
        // RT, but stw must store only the low 32 bits. Locks the
        // implicit `buffer_store(_, _, _, 4, val)` low-bytes
        // contract so a future change cannot silently store the
        // sign-extended high half.
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::LiStw {
                rt: 3,
                imm: -1,
                ra_store: 1,
                store_offset: 0,
            },
            &mut s,
            0,
            &[0u8; 0x2000],
            &mut effects,
        );
        assert_eq!(s.gpr[3], u64::MAX);
        let stored = effects
            .iter()
            .find_map(|e| match e {
                Effect::SharedWriteIntent { range, bytes, .. } if range.start().raw() == 0x1000 => {
                    Some(bytes.bytes().to_vec())
                }
                _ => None,
            })
            .expect("LiStw must emit a store at 0x1000");
        assert_eq!(stored, vec![0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(stored.len(), 4, "stw stores 4 bytes, not 8");
    }

    #[test]
    fn cmpw_zero_positive() {
        let mut s = PpuState::new();
        s.gpr[3] = 5;
        exec_no_mem(&PpuInstruction::CmpwZero { bf: 0, ra: 3 }, &mut s);
        assert_eq!(s.cr_field(0), 0b0100); // GT
    }

    #[test]
    fn cmpw_zero_propagates_xer_so() {
        let mut s = PpuState::new();
        s.gpr[3] = 0;
        s.set_xer_ov(true); // set sticky SO
        exec_no_mem(&PpuInstruction::CmpwZero { bf: 0, ra: 3 }, &mut s);
        // EQ + SO = 0b0010 | 0b0001 = 0b0011.
        assert_eq!(s.cr_field(0), 0b0011);
    }

    #[test]
    fn lwz_cmpwi_propagates_xer_so() {
        let mut mem = vec![0u8; 0x2000];
        mem[0x100..0x104].copy_from_slice(&5u32.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.set_xer_ov(true);
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::LwzCmpwi {
                rt: 3,
                ra_load: 1,
                offset: 0,
                bf: 2,
                cmp_imm: 10,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        // LT + SO.
        assert_eq!(s.cr_field(2), 0b1001);
    }

    #[test]
    fn lwz_cmpwi_loaded_value_negative_as_i32() {
        // lwz zero-extends in the GPR but cmpwi treats the low 32
        // bits as signed. A loaded 0xFFFF_FFFF must compare as -1,
        // not 4294967295.
        let mut mem = vec![0u8; 0x2000];
        mem[0x100..0x104].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::LwzCmpwi {
                rt: 3,
                ra_load: 1,
                offset: 0,
                bf: 0,
                cmp_imm: 1,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3], 0xFFFF_FFFF, "lwz zero-extends in the GPR");
        assert_eq!(s.cr_field(0), 0b1000, "cmpwi sees -1, less than 1");
    }

    #[test]
    fn cmpwi_bc_propagates_xer_so_when_not_taken() {
        let mut s = PpuState::new();
        s.pc = 0x1000;
        s.gpr[3] = 5;
        s.set_xer_ov(true);
        let v = exec_no_mem(
            &PpuInstruction::CmpwiBc {
                bf: 0,
                ra: 3,
                imm: 10,
                bo: 0x0C,
                bi: 1, // GT bit -- compare result is LT, so not taken
                target_offset: 16,
            },
            &mut s,
        );
        assert_eq!(v, ExecuteVerdict::Continue);
        assert_eq!(s.cr_field(0), 0b1001, "LT + SO");
    }

    #[test]
    fn cmpwi_bc_negative_target_offset_branches_backward() {
        let mut s = PpuState::new();
        s.pc = 0x2000;
        s.gpr[3] = 0;
        let v = exec_no_mem(
            &PpuInstruction::CmpwiBc {
                bf: 0,
                ra: 3,
                imm: 0,
                bo: 0x0C,
                bi: 2, // EQ
                target_offset: -16,
            },
            &mut s,
        );
        assert_eq!(v, ExecuteVerdict::Branch);
        // Target is (super + 4) + offset = 0x2004 + (-16) = 0x1FF4.
        assert_eq!(s.pc, 0x1FF4);
    }

    #[test]
    fn mflr_std_writes_lr_then_stores_64_bits() {
        let mut s = PpuState::new();
        s.lr = 0xDEAD_BEEF_CAFE_BABE;
        s.gpr[1] = 0x1000;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::MflrStd {
                rt: 3,
                ra_store: 1,
                store_offset: 0,
            },
            &mut s,
            0,
            &[0u8; 0x2000],
            &mut effects,
        );
        assert_eq!(s.gpr[3], 0xDEAD_BEEF_CAFE_BABE);
        let stored = effects
            .iter()
            .find_map(|e| match e {
                Effect::SharedWriteIntent { range, bytes, .. } if range.start().raw() == 0x1000 => {
                    Some((range.length(), bytes.bytes().to_vec()))
                }
                _ => None,
            })
            .expect("MflrStd must emit a store");
        assert_eq!(stored.0, 8, "std stores 8 bytes");
        assert_eq!(
            stored.1,
            vec![0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE]
        );
    }

    #[test]
    fn ld_mtlr_loads_into_rt_and_lr() {
        let mut mem = vec![0u8; 0x2000];
        mem[0x100..0x108].copy_from_slice(&0xCAFE_BABE_DEAD_BEEFu64.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        let mut effects = Vec::new();
        let v = exec_with_mem(
            &PpuInstruction::LdMtlr {
                rt: 3,
                ra_load: 1,
                offset: 0,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(v, ExecuteVerdict::Continue);
        assert_eq!(s.gpr[3], 0xCAFE_BABE_DEAD_BEEF);
        assert_eq!(s.lr, 0xCAFE_BABE_DEAD_BEEF);
    }
}
