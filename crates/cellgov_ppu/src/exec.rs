//! PPU instruction execution.
//!
//! Takes a decoded `PpuInstruction` and a `PpuState`, applies the
//! instruction's semantics (register mutation), and returns a
//! `PpuStepOutcome`. Syscall dispatch is handled here (or delegated
//! to `syscall.rs`).

use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_effects::Effect;
use cellgov_exec::YieldReason;

/// What happened after executing one instruction.
#[derive(Debug)]
pub enum PpuStepOutcome {
    /// Instruction executed, advance PC by 4, keep running.
    Continue,
    /// PC was set explicitly (branch taken). Do not advance PC.
    Branch,
    /// Instruction requires runtime mediation.
    Yield {
        /// Effects to commit.
        effects: Vec<Effect>,
        /// Why the unit is yielding.
        reason: YieldReason,
    },
    /// Load request: run_until_yield reads `size` bytes from `ea` in
    /// guest memory and zero-extends into GPR `rt`.
    Load {
        /// Guest effective address.
        ea: u64,
        /// Number of bytes (1, 4, or 8).
        size: u8,
        /// Destination register.
        rt: u8,
    },
    /// Store request: run_until_yield emits a SharedWriteIntent for
    /// `size` bytes of `value` at `ea`.
    Store {
        /// Guest effective address.
        ea: u64,
        /// Number of bytes (1, 4, or 8).
        size: u8,
        /// Value to store (right-justified).
        value: u64,
    },
    /// 16-byte vector store request: run_until_yield emits a
    /// SharedWriteIntent for the 16 big-endian bytes of `value` at
    /// `ea`. The effective address is already aligned by the
    /// instruction semantics (stvx forces 16-byte alignment).
    StoreVec {
        /// Guest effective address (already 16-byte aligned).
        ea: u64,
        /// 128-bit vector value (interpreted big-endian on store).
        value: u128,
    },
    /// Syscall: run_until_yield handles dispatch. The syscall number
    /// is in r11 per LV2 convention.
    Syscall,
    /// Instruction caused an architecture fault.
    Fault(PpuFault),
}

/// PPU-specific fault categories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PpuFault {
    /// PC is outside addressable memory.
    PcOutOfRange(u64),
    /// Memory access at an invalid address.
    InvalidAddress(u64),
    /// Unsupported syscall number.
    UnsupportedSyscall(u64),
}

/// Execute a single decoded PPU instruction against the given state.
pub fn execute(
    insn: &PpuInstruction,
    state: &mut PpuState,
    _unit_id: cellgov_event::UnitId,
) -> PpuStepOutcome {
    match *insn {
        // =================================================================
        // Integer loads (delegate to run_until_yield via Load outcome)
        // =================================================================
        PpuInstruction::Lwz { rt, ra, imm } => PpuStepOutcome::Load {
            ea: state.ea_d_form(ra, imm),
            size: 4,
            rt,
        },
        PpuInstruction::Lbz { rt, ra, imm } => PpuStepOutcome::Load {
            ea: state.ea_d_form(ra, imm),
            size: 1,
            rt,
        },
        PpuInstruction::Ld { rt, ra, imm } => PpuStepOutcome::Load {
            ea: state.ea_d_form(ra, imm),
            size: 8,
            rt,
        },

        // =================================================================
        // Integer stores (delegate to run_until_yield via Store outcome)
        // =================================================================
        PpuInstruction::Stw { rs, ra, imm } => PpuStepOutcome::Store {
            ea: state.ea_d_form(ra, imm),
            size: 4,
            value: state.gpr[rs as usize],
        },
        PpuInstruction::Stb { rs, ra, imm } => PpuStepOutcome::Store {
            ea: state.ea_d_form(ra, imm),
            size: 1,
            value: state.gpr[rs as usize],
        },
        PpuInstruction::Sth { rs, ra, imm } => PpuStepOutcome::Store {
            ea: state.ea_d_form(ra, imm),
            size: 2,
            value: state.gpr[rs as usize],
        },
        PpuInstruction::Std { rs, ra, imm } => PpuStepOutcome::Store {
            ea: state.ea_d_form(ra, imm),
            size: 8,
            value: state.gpr[rs as usize],
        },
        PpuInstruction::Stwu { rs, ra, imm } => {
            // stwu: store then update. ra must not be 0 per PPC spec;
            // the update happens regardless of store success.
            let ea = state.ea_d_form(ra, imm);
            let value = state.gpr[rs as usize];
            state.gpr[ra as usize] = ea;
            PpuStepOutcome::Store { ea, size: 4, value }
        }
        PpuInstruction::Stdu { rs, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            let value = state.gpr[rs as usize];
            state.gpr[ra as usize] = ea;
            PpuStepOutcome::Store { ea, size: 8, value }
        }

        // =================================================================
        // Integer arithmetic / logical
        // =================================================================
        PpuInstruction::Addi { rt, ra, imm } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            state.gpr[rt as usize] = base.wrapping_add(imm as i64 as u64);
            PpuStepOutcome::Continue
        }
        PpuInstruction::Addis { rt, ra, imm } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            state.gpr[rt as usize] = base.wrapping_add((imm as i64 as u64) << 16);
            PpuStepOutcome::Continue
        }
        PpuInstruction::Add { rt, ra, rb } => {
            state.gpr[rt as usize] = state.gpr[ra as usize].wrapping_add(state.gpr[rb as usize]);
            PpuStepOutcome::Continue
        }
        PpuInstruction::Or { ra, rs, rb } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] | state.gpr[rb as usize];
            PpuStepOutcome::Continue
        }
        PpuInstruction::Extsw { ra, rs } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] as i32 as i64 as u64;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Ori { ra, rs, imm } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] | imm as u64;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Oris { ra, rs, imm } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] | ((imm as u64) << 16);
            PpuStepOutcome::Continue
        }

        // =================================================================
        // Compare
        // =================================================================
        PpuInstruction::Cmpwi { bf, ra, imm } => {
            let a = state.gpr[ra as usize] as i32;
            let b = imm as i32;
            let cr_val = if a < b {
                0b1000
            } else if a > b {
                0b0100
            } else {
                0b0010
            };
            state.set_cr_field(bf, cr_val);
            PpuStepOutcome::Continue
        }
        PpuInstruction::Cmplwi { bf, ra, imm } => {
            let a = state.gpr[ra as usize] as u32;
            let b = imm as u32;
            let cr_val = if a < b {
                0b1000
            } else if a > b {
                0b0100
            } else {
                0b0010
            };
            state.set_cr_field(bf, cr_val);
            PpuStepOutcome::Continue
        }

        // =================================================================
        // Branch
        // =================================================================
        PpuInstruction::B { offset, link } => {
            if link {
                state.lr = state.pc + 4;
            }
            state.pc = (state.pc as i64).wrapping_add(offset as i64) as u64;
            PpuStepOutcome::Branch
        }
        PpuInstruction::Bc {
            bo,
            bi,
            offset,
            link,
        } => {
            if link {
                state.lr = state.pc + 4;
            }
            if branch_condition(state, bo, bi) {
                state.pc = (state.pc as i64).wrapping_add(offset as i64) as u64;
                PpuStepOutcome::Branch
            } else {
                PpuStepOutcome::Continue
            }
        }
        PpuInstruction::Bclr { bo, bi, link } => {
            let target = state.lr & !3;
            if link {
                state.lr = state.pc + 4;
            }
            if branch_condition(state, bo, bi) {
                state.pc = target;
                PpuStepOutcome::Branch
            } else {
                PpuStepOutcome::Continue
            }
        }
        PpuInstruction::Bcctr { bo, bi, link } => {
            if link {
                state.lr = state.pc + 4;
            }
            if branch_condition(state, bo, bi) {
                state.pc = state.ctr & !3;
                PpuStepOutcome::Branch
            } else {
                PpuStepOutcome::Continue
            }
        }

        // =================================================================
        // Special-purpose register moves
        // =================================================================
        PpuInstruction::Mflr { rt } => {
            state.gpr[rt as usize] = state.lr;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Mtlr { rs } => {
            state.lr = state.gpr[rs as usize];
            PpuStepOutcome::Continue
        }
        PpuInstruction::Mtctr { rs } => {
            state.ctr = state.gpr[rs as usize];
            PpuStepOutcome::Continue
        }

        // =================================================================
        // Rotate / mask
        // =================================================================
        PpuInstruction::Rlwinm { ra, rs, sh, mb, me } => {
            let val = state.gpr[rs as usize] as u32;
            let rotated = val.rotate_left(sh as u32);
            let mask = rlwinm_mask(mb, me);
            state.gpr[ra as usize] = (rotated & mask) as u64;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Rldicl { ra, rs, sh, mb } => {
            let rotated = state.gpr[rs as usize].rotate_left(sh as u32);
            state.gpr[ra as usize] = rotated & mask64(mb, 63);
            PpuStepOutcome::Continue
        }
        PpuInstruction::Rldicr { ra, rs, sh, me } => {
            let rotated = state.gpr[rs as usize].rotate_left(sh as u32);
            state.gpr[ra as usize] = rotated & mask64(0, me);
            PpuStepOutcome::Continue
        }

        // =================================================================
        // Vector (AltiVec / VMX)
        // =================================================================
        PpuInstruction::Vxor { vt, va, vb } => {
            state.vr[vt as usize] = state.vr[va as usize] ^ state.vr[vb as usize];
            PpuStepOutcome::Continue
        }
        PpuInstruction::Stvx { vs, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]) & !15u64;
            PpuStepOutcome::StoreVec {
                ea,
                value: state.vr[vs as usize],
            }
        }

        // =================================================================
        // System call
        // =================================================================
        PpuInstruction::Sc => PpuStepOutcome::Syscall,
    }
}

/// Evaluate a PPC branch condition (BO/BI fields).
///
/// BO encoding (5 bits):
/// - bit 0 (0x10): if set, do not test CR
/// - bit 1 (0x08): CR condition to test against (true/false)
/// - bit 2 (0x04): if set, do not decrement CTR
/// - bit 3 (0x02): CTR condition (branch if CTR==0 vs !=0)
/// - bit 4 (0x01): branch prediction hint (ignored)
fn branch_condition(state: &mut PpuState, bo: u8, bi: u8) -> bool {
    let decr_ctr = (bo & 0x04) == 0;
    if decr_ctr {
        state.ctr = state.ctr.wrapping_sub(1);
    }

    let ctr_ok = (bo & 0x04) != 0 || ((state.ctr != 0) ^ ((bo & 0x02) != 0));
    let cr_ok = (bo & 0x10) != 0 || (state.cr_bit(bi) == ((bo & 0x08) != 0));

    ctr_ok && cr_ok
}

/// Compute the 32-bit mask for rlwinm given MB and ME fields.
fn rlwinm_mask(mb: u8, me: u8) -> u32 {
    if mb <= me {
        // Contiguous mask from bit mb to bit me (inclusive)
        let top = 0xFFFF_FFFFu32 >> mb;
        let bottom = 0xFFFF_FFFFu32 << (31 - me);
        top & bottom
    } else {
        // Wrapped mask: bits [0..me] and [mb..31]
        let top = 0xFFFF_FFFFu32 << (31 - me);
        let bottom = 0xFFFF_FFFFu32 >> mb;
        top | bottom
    }
}

/// Compute a 64-bit PPC mask from `mb` to `me` (inclusive). Bit 0 is
/// the MSB. When mb > me the mask is the wrapped complement (bits
/// 0..me and mb..63). Used by rldicl / rldicr / rldic.
fn mask64(mb: u8, me: u8) -> u64 {
    let all = 0xFFFF_FFFF_FFFF_FFFFu64;
    if mb <= me {
        let top = all >> mb;
        let bottom = all << (63 - me);
        top & bottom
    } else {
        let top = all << (63 - me);
        let bottom = all >> mb;
        top | bottom
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_event::UnitId;

    fn uid() -> UnitId {
        UnitId::new(0)
    }

    #[test]
    fn addi_with_ra_zero_is_li() {
        let mut s = PpuState::new();
        execute(
            &PpuInstruction::Addi {
                rt: 3,
                ra: 0,
                imm: 42,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.gpr[3], 42);
    }

    #[test]
    fn addi_with_ra_nonzero_adds() {
        let mut s = PpuState::new();
        s.gpr[5] = 100;
        execute(
            &PpuInstruction::Addi {
                rt: 3,
                ra: 5,
                imm: -10,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.gpr[3], 90);
    }

    #[test]
    fn addis_shifts_left_16() {
        let mut s = PpuState::new();
        execute(
            &PpuInstruction::Addis {
                rt: 3,
                ra: 0,
                imm: 1,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.gpr[3], 0x10000);
    }

    #[test]
    fn ori_zero_is_move() {
        let mut s = PpuState::new();
        s.gpr[5] = 0xCAFE;
        execute(
            &PpuInstruction::Ori {
                ra: 3,
                rs: 5,
                imm: 0,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.gpr[3], 0xCAFE);
    }

    #[test]
    fn cmpwi_sets_cr_field() {
        let mut s = PpuState::new();
        s.gpr[3] = 10;
        execute(
            &PpuInstruction::Cmpwi {
                bf: 0,
                ra: 3,
                imm: 10,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.cr_field(0), 0b0010); // EQ
    }

    #[test]
    fn branch_unconditional() {
        let mut s = PpuState::new();
        s.pc = 0x1000;
        let result = execute(
            &PpuInstruction::B {
                offset: -8,
                link: false,
            },
            &mut s,
            uid(),
        );
        assert!(matches!(result, PpuStepOutcome::Branch));
        assert_eq!(s.pc, 0x0FF8);
    }

    #[test]
    fn bl_sets_lr() {
        let mut s = PpuState::new();
        s.pc = 0x1000;
        execute(
            &PpuInstruction::B {
                offset: 0x100,
                link: true,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.lr, 0x1004);
        assert_eq!(s.pc, 0x1100);
    }

    #[test]
    fn blr_returns_to_lr() {
        let mut s = PpuState::new();
        s.pc = 0x2000;
        s.lr = 0x1000;
        // BO=0x14 = always taken (don't test CR, don't decr CTR)
        let result = execute(
            &PpuInstruction::Bclr {
                bo: 0x14,
                bi: 0,
                link: false,
            },
            &mut s,
            uid(),
        );
        assert!(matches!(result, PpuStepOutcome::Branch));
        assert_eq!(s.pc, 0x1000);
    }

    #[test]
    fn mflr_mtlr_roundtrip() {
        let mut s = PpuState::new();
        s.gpr[5] = 0xABCD;
        execute(&PpuInstruction::Mtlr { rs: 5 }, &mut s, uid());
        assert_eq!(s.lr, 0xABCD);
        execute(&PpuInstruction::Mflr { rt: 3 }, &mut s, uid());
        assert_eq!(s.gpr[3], 0xABCD);
    }

    #[test]
    fn rlwinm_slwi() {
        let mut s = PpuState::new();
        s.gpr[5] = 0x0001;
        // slwi r3, r5, 16 = rlwinm r3, r5, 16, 0, 15
        execute(
            &PpuInstruction::Rlwinm {
                ra: 3,
                rs: 5,
                sh: 16,
                mb: 0,
                me: 15,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.gpr[3], 0x10000);
    }

    #[test]
    fn rlwinm_mask_contiguous() {
        assert_eq!(rlwinm_mask(0, 31), 0xFFFFFFFF);
        assert_eq!(rlwinm_mask(16, 31), 0x0000FFFF);
        assert_eq!(rlwinm_mask(0, 15), 0xFFFF0000);
    }

    #[test]
    fn rlwinm_mask_wrapped() {
        // Wrapped: bits [0..3] and [28..31]
        assert_eq!(rlwinm_mask(28, 3), 0xF000000F);
    }

    #[test]
    fn vxor_self_zeros_vector_register() {
        let mut s = PpuState::new();
        s.vr[5] = 0xDEAD_BEEF_DEAD_BEEF_DEAD_BEEF_DEAD_BEEFu128;
        execute(
            &PpuInstruction::Vxor {
                vt: 5,
                va: 5,
                vb: 5,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.vr[5], 0);
    }

    #[test]
    fn stvx_aligns_ea_and_carries_vector_value() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        s.gpr[8] = 0x1F;
        s.vr[0] = 0xAABB_CCDD_EEFF_0011_2233_4455_6677_8899u128;
        let result = execute(
            &PpuInstruction::Stvx {
                vs: 0,
                ra: 1,
                rb: 8,
            },
            &mut s,
            uid(),
        );
        match result {
            PpuStepOutcome::StoreVec { ea, value } => {
                // 0x1000 + 0x1F = 0x101F, aligned down to 0x1010.
                assert_eq!(ea, 0x1010);
                assert_eq!(value, 0xAABB_CCDD_EEFF_0011_2233_4455_6677_8899u128);
            }
            other => panic!("expected StoreVec, got {:?}", other),
        }
    }

    #[test]
    fn extsw_sign_extends_low_32_bits() {
        let mut s = PpuState::new();
        s.gpr[3] = 0x0000_0000_8000_0000; // bit 31 set in low word
        execute(&PpuInstruction::Extsw { ra: 4, rs: 3 }, &mut s, uid());
        assert_eq!(s.gpr[4], 0xFFFF_FFFF_8000_0000);
    }

    #[test]
    fn sc_returns_syscall() {
        let mut s = PpuState::new();
        let result = execute(&PpuInstruction::Sc, &mut s, uid());
        assert!(matches!(result, PpuStepOutcome::Syscall));
    }

    #[test]
    fn lwz_returns_load() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        let result = execute(
            &PpuInstruction::Lwz {
                rt: 3,
                ra: 1,
                imm: 8,
            },
            &mut s,
            uid(),
        );
        match result {
            PpuStepOutcome::Load { ea, size, rt } => {
                assert_eq!(ea, 0x1008);
                assert_eq!(size, 4);
                assert_eq!(rt, 3);
            }
            other => panic!("expected Load, got {:?}", other),
        }
    }

    #[test]
    fn stw_returns_store() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        s.gpr[5] = 0xDEADBEEF;
        let result = execute(
            &PpuInstruction::Stw {
                rs: 5,
                ra: 1,
                imm: 0,
            },
            &mut s,
            uid(),
        );
        match result {
            PpuStepOutcome::Store { ea, size, value } => {
                assert_eq!(ea, 0x1000);
                assert_eq!(size, 4);
                assert_eq!(value, 0xDEADBEEF);
            }
            other => panic!("expected Store, got {:?}", other),
        }
    }

    #[test]
    fn bc_beq_taken() {
        let mut s = PpuState::new();
        s.pc = 0x1000;
        s.set_cr_field(0, 0b0010); // EQ set
                                   // beq cr0, +8: BO=0x0C (test CR, don't decr CTR), BI=2 (EQ bit of cr0)
        let result = execute(
            &PpuInstruction::Bc {
                bo: 0x0C,
                bi: 2,
                offset: 8,
                link: false,
            },
            &mut s,
            uid(),
        );
        assert!(matches!(result, PpuStepOutcome::Branch));
        assert_eq!(s.pc, 0x1008);
    }

    #[test]
    fn bc_beq_not_taken() {
        let mut s = PpuState::new();
        s.pc = 0x1000;
        s.set_cr_field(0, 0b0100); // GT set, not EQ
        let result = execute(
            &PpuInstruction::Bc {
                bo: 0x0C,
                bi: 2,
                offset: 8,
                link: false,
            },
            &mut s,
            uid(),
        );
        assert!(matches!(result, PpuStepOutcome::Continue));
        assert_eq!(s.pc, 0x1000); // unchanged
    }
}
