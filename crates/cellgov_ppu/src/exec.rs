//! PPU instruction execution.
//!
//! Takes a decoded `PpuInstruction` and a `PpuState`, applies the
//! instruction's semantics (register mutation), and returns a
//! `PpuStepOutcome`. Syscall dispatch is handled here (or delegated
//! to `syscall.rs`).

use crate::fp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_effects::Effect;
use cellgov_exec::YieldReason;

/// What happened after executing one instruction.
#[derive(Debug)]
#[allow(missing_docs)]
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
    /// Sign-extending load: run_until_yield reads `size` bytes from `ea`
    /// and sign-extends the result to 64 bits before writing to GPR
    /// `rt`. Used by lha/lhax (halfword algebraic) and lwa/lwax
    /// (word algebraic) which differ from their zero-extending
    /// counterparts only in how they widen into the 64-bit register.
    LoadSigned {
        /// Guest effective address.
        ea: u64,
        /// Number of bytes (1, 2, or 4).
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
    /// 16-byte vector load request.
    LoadVec { ea: u64, vt: u8 },
    /// Floating-point load: run_until_yield reads `size` bytes from
    /// `ea` and writes the result to `FPR\[frt\]`.
    FpLoad {
        ea: u64,
        /// 4 for lfs, 8 for lfd.
        size: u8,
        frt: u8,
    },
    /// Floating-point store: run_until_yield writes `size` bytes of
    /// the FPR value at `ea`.
    FpStore {
        ea: u64,
        /// 4 for stfs, 8 for stfd.
        size: u8,
        /// Raw f64 bits from the FPR.
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
        PpuInstruction::Lhz { rt, ra, imm } => PpuStepOutcome::Load {
            ea: state.ea_d_form(ra, imm),
            size: 2,
            rt,
        },
        PpuInstruction::Lha { rt, ra, imm } => PpuStepOutcome::LoadSigned {
            ea: state.ea_d_form(ra, imm),
            size: 2,
            rt,
        },
        PpuInstruction::Lwzu { rt, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            state.gpr[ra as usize] = ea;
            PpuStepOutcome::Load { ea, size: 4, rt }
        }
        PpuInstruction::Lbzu { rt, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            state.gpr[ra as usize] = ea;
            PpuStepOutcome::Load { ea, size: 1, rt }
        }
        PpuInstruction::Ldu { rt, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            state.gpr[ra as usize] = ea;
            PpuStepOutcome::Load { ea, size: 8, rt }
        }
        PpuInstruction::Ld { rt, ra, imm } => PpuStepOutcome::Load {
            ea: state.ea_d_form(ra, imm),
            size: 8,
            rt,
        },
        // Indexed loads
        PpuInstruction::Lwzx { rt, ra, rb } => PpuStepOutcome::Load {
            ea: state.ea_x_form(ra, rb),
            size: 4,
            rt,
        },
        PpuInstruction::Lbzx { rt, ra, rb } => PpuStepOutcome::Load {
            ea: state.ea_x_form(ra, rb),
            size: 1,
            rt,
        },
        PpuInstruction::Ldx { rt, ra, rb } => PpuStepOutcome::Load {
            ea: state.ea_x_form(ra, rb),
            size: 8,
            rt,
        },
        PpuInstruction::Lhzx { rt, ra, rb } => PpuStepOutcome::Load {
            ea: state.ea_x_form(ra, rb),
            size: 2,
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
        // Indexed stores
        PpuInstruction::Stwx { rs, ra, rb } => PpuStepOutcome::Store {
            ea: state.ea_x_form(ra, rb),
            size: 4,
            value: state.gpr[rs as usize],
        },
        PpuInstruction::Stdx { rs, ra, rb } => PpuStepOutcome::Store {
            ea: state.ea_x_form(ra, rb),
            size: 8,
            value: state.gpr[rs as usize],
        },
        PpuInstruction::Ldarx { rt, ra, rb } => PpuStepOutcome::Load {
            ea: state.ea_x_form(ra, rb),
            size: 8,
            rt,
        },
        PpuInstruction::Stdcx { rs, ra, rb } => {
            // Single-threaded: reservation never lost, CAS always succeeds.
            state.set_cr_field(0, 0b0010); // EQ
            PpuStepOutcome::Store {
                ea: state.ea_x_form(ra, rb),
                size: 8,
                value: state.gpr[rs as usize],
            }
        }
        PpuInstruction::Lwarx { rt, ra, rb } => PpuStepOutcome::Load {
            ea: state.ea_x_form(ra, rb),
            size: 4,
            rt,
        },
        PpuInstruction::Stwcx { rs, ra, rb } => {
            // Single-threaded: reservation never lost, CAS always succeeds.
            state.set_cr_field(0, 0b0010); // EQ
            PpuStepOutcome::Store {
                ea: state.ea_x_form(ra, rb),
                size: 4,
                value: state.gpr[rs as usize],
            }
        }
        PpuInstruction::Stbx { rs, ra, rb } => PpuStepOutcome::Store {
            ea: state.ea_x_form(ra, rb),
            size: 1,
            value: state.gpr[rs as usize],
        },

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
        PpuInstruction::Subfic { rt, ra, imm } => {
            let a = state.gpr[ra as usize];
            let b = imm as i64 as u64;
            state.gpr[rt as usize] = b.wrapping_sub(a);
            PpuStepOutcome::Continue
        }
        PpuInstruction::Mulli { rt, ra, imm } => {
            let a = state.gpr[ra as usize] as i64;
            let b = imm as i64;
            state.gpr[rt as usize] = a.wrapping_mul(b) as u64;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Addic { rt, ra, imm } => {
            let a = state.gpr[ra as usize];
            let b = imm as i64 as u64;
            state.gpr[rt as usize] = a.wrapping_add(b);
            // CA bit would be set here but we don't track XER yet
            PpuStepOutcome::Continue
        }
        PpuInstruction::Add { rt, ra, rb } => {
            state.gpr[rt as usize] = state.gpr[ra as usize].wrapping_add(state.gpr[rb as usize]);
            PpuStepOutcome::Continue
        }
        PpuInstruction::Subf { rt, ra, rb } => {
            state.gpr[rt as usize] = state.gpr[rb as usize].wrapping_sub(state.gpr[ra as usize]);
            PpuStepOutcome::Continue
        }
        PpuInstruction::Neg { rt, ra } => {
            state.gpr[rt as usize] = (state.gpr[ra as usize] as i64).wrapping_neg() as u64;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Mullw { rt, ra, rb } => {
            let a = state.gpr[ra as usize] as i32;
            let b = state.gpr[rb as usize] as i32;
            state.gpr[rt as usize] = (a as i64).wrapping_mul(b as i64) as u64;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Mulhwu { rt, ra, rb } => {
            let a = state.gpr[ra as usize] as u32 as u64;
            let b = state.gpr[rb as usize] as u32 as u64;
            state.gpr[rt as usize] = (a * b) >> 32;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Mulhdu { rt, ra, rb } => {
            let a = state.gpr[ra as usize] as u128;
            let b = state.gpr[rb as usize] as u128;
            state.gpr[rt as usize] = ((a * b) >> 64) as u64;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Adde { rt, ra, rb } => {
            let a = state.gpr[ra as usize];
            let b = state.gpr[rb as usize];
            let ca_in: u64 = state.xer_ca() as u64;
            let (sum1, c1) = a.overflowing_add(b);
            let (sum2, c2) = sum1.overflowing_add(ca_in);
            state.gpr[rt as usize] = sum2;
            state.set_xer_ca(c1 || c2);
            PpuStepOutcome::Continue
        }
        PpuInstruction::Divw { rt, ra, rb } => {
            let a = state.gpr[ra as usize] as i32;
            let b = state.gpr[rb as usize] as i32;
            let result = if b == 0 { 0 } else { a.wrapping_div(b) };
            state.gpr[rt as usize] = result as i64 as u64;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Divwu { rt, ra, rb } => {
            let a = state.gpr[ra as usize] as u32;
            let b = state.gpr[rb as usize] as u32;
            let result = if b == 0 { 0 } else { a / b };
            state.gpr[rt as usize] = result as u64;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Divd { rt, ra, rb } => {
            let a = state.gpr[ra as usize] as i64;
            let b = state.gpr[rb as usize] as i64;
            let result = if b == 0 { 0 } else { a.wrapping_div(b) };
            state.gpr[rt as usize] = result as u64;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Divdu { rt, ra, rb } => {
            let a = state.gpr[ra as usize];
            let b = state.gpr[rb as usize];
            let result = if b == 0 { 0 } else { a / b };
            state.gpr[rt as usize] = result;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Mulld { rt, ra, rb } => {
            let a = state.gpr[ra as usize];
            let b = state.gpr[rb as usize];
            state.gpr[rt as usize] = a.wrapping_mul(b);
            PpuStepOutcome::Continue
        }
        PpuInstruction::Or { ra, rs, rb } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] | state.gpr[rb as usize];
            PpuStepOutcome::Continue
        }
        PpuInstruction::And { ra, rs, rb } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] & state.gpr[rb as usize];
            PpuStepOutcome::Continue
        }
        PpuInstruction::Nor { ra, rs, rb } => {
            state.gpr[ra as usize] = !(state.gpr[rs as usize] | state.gpr[rb as usize]);
            PpuStepOutcome::Continue
        }
        PpuInstruction::Andc { ra, rs, rb } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] & !state.gpr[rb as usize];
            PpuStepOutcome::Continue
        }
        PpuInstruction::Xor { ra, rs, rb } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] ^ state.gpr[rb as usize];
            PpuStepOutcome::Continue
        }
        PpuInstruction::AndiDot { ra, rs, imm } => {
            let result = state.gpr[rs as usize] & imm as u64;
            state.gpr[ra as usize] = result;
            // andi. always updates CR0
            let cr_val = if (result as i64) < 0 {
                0b1000
            } else if result > 0 {
                0b0100
            } else {
                0b0010
            };
            state.set_cr_field(0, cr_val);
            PpuStepOutcome::Continue
        }
        PpuInstruction::Slw { ra, rs, rb } => {
            let shift = state.gpr[rb as usize] & 0x3F;
            let val = state.gpr[rs as usize] as u32;
            let result = if shift < 32 { val << shift } else { 0 };
            state.gpr[ra as usize] = result as u64;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Srw { ra, rs, rb } => {
            let shift = state.gpr[rb as usize] & 0x3F;
            let val = state.gpr[rs as usize] as u32;
            let result = if shift < 32 { val >> shift } else { 0 };
            state.gpr[ra as usize] = result as u64;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Srawi { ra, rs, sh } => {
            let val = state.gpr[rs as usize] as i32;
            let result = val >> sh;
            state.gpr[ra as usize] = result as i64 as u64;
            // CA bit would be set here but we don't track XER yet
            PpuStepOutcome::Continue
        }
        PpuInstruction::Sld { ra, rs, rb } => {
            let shift = state.gpr[rb as usize] & 0x7F;
            let result = if shift < 64 {
                state.gpr[rs as usize] << shift
            } else {
                0
            };
            state.gpr[ra as usize] = result;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Srd { ra, rs, rb } => {
            let shift = state.gpr[rb as usize] & 0x7F;
            let result = if shift < 64 {
                state.gpr[rs as usize] >> shift
            } else {
                0
            };
            state.gpr[ra as usize] = result;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Cntlzw { ra, rs } => {
            let val = state.gpr[rs as usize] as u32;
            state.gpr[ra as usize] = val.leading_zeros() as u64;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Extsh { ra, rs } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] as i16 as i64 as u64;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Extsb { ra, rs } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] as i8 as i64 as u64;
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
        PpuInstruction::Xori { ra, rs, imm } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] ^ imm as u64;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Xoris { ra, rs, imm } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] ^ ((imm as u64) << 16);
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
        PpuInstruction::Cmpw { bf, ra, rb } => {
            let a = state.gpr[ra as usize] as i32;
            let b = state.gpr[rb as usize] as i32;
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
        PpuInstruction::Cmplw { bf, ra, rb } => {
            let a = state.gpr[ra as usize] as u32;
            let b = state.gpr[rb as usize] as u32;
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
        PpuInstruction::Cmpd { bf, ra, rb } => {
            let a = state.gpr[ra as usize] as i64;
            let b = state.gpr[rb as usize] as i64;
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
        PpuInstruction::Cmpld { bf, ra, rb } => {
            let a = state.gpr[ra as usize];
            let b = state.gpr[rb as usize];
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
        PpuInstruction::Mftb { rt } => {
            state.tb += 512; // advance deterministically per read
            state.gpr[rt as usize] = state.tb;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Mfcr { rt } => {
            state.gpr[rt as usize] = state.cr as u64;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Mtcrf { rs, crm } => {
            let val = (state.gpr[rs as usize] >> 32) as u32;
            // Each bit in CRM selects a 4-bit CR field.
            for i in 0..8u8 {
                if crm & (1 << (7 - i)) != 0 {
                    let shift = (7 - i) * 4;
                    let field_bits = (val >> shift) & 0xF;
                    let mask = 0xF << shift;
                    state.cr = (state.cr & !mask) | (field_bits << shift);
                }
            }
            PpuStepOutcome::Continue
        }
        PpuInstruction::Mflr { rt } => {
            state.gpr[rt as usize] = state.lr;
            PpuStepOutcome::Continue
        }
        PpuInstruction::Mtlr { rs } => {
            state.lr = state.gpr[rs as usize];
            PpuStepOutcome::Continue
        }
        PpuInstruction::Mfctr { rt } => {
            state.gpr[rt as usize] = state.ctr;
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
        PpuInstruction::Rlwnm { ra, rs, rb, mb, me } => {
            let val = state.gpr[rs as usize] as u32;
            let n = (state.gpr[rb as usize] & 0x1F) as u32;
            let rotated = val.rotate_left(n);
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
        PpuInstruction::Vx { xo, vt, va, vb } => crate::exec_vec::execute_vx(state, xo, vt, va, vb),
        PpuInstruction::Va { xo, vt, va, vb, vc } => {
            crate::exec_vec::execute_va(state, xo, vt, va, vb, vc)
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
        // =================================================================
        // Floating-point loads/stores
        // =================================================================
        PpuInstruction::Lfs { frt, ra, imm } => PpuStepOutcome::FpLoad {
            ea: state.ea_d_form(ra, imm),
            size: 4,
            frt,
        },
        PpuInstruction::Lfd { frt, ra, imm } => PpuStepOutcome::FpLoad {
            ea: state.ea_d_form(ra, imm),
            size: 8,
            frt,
        },
        PpuInstruction::Stfs { frs, ra, imm } => PpuStepOutcome::FpStore {
            ea: state.ea_d_form(ra, imm),
            size: 4,
            value: state.fpr[frs as usize],
        },
        PpuInstruction::Stfd { frs, ra, imm } => PpuStepOutcome::FpStore {
            ea: state.ea_d_form(ra, imm),
            size: 8,
            value: state.fpr[frs as usize],
        },

        // =================================================================
        // Floating-point arithmetic (opcode 63, double precision)
        // =================================================================
        PpuInstruction::Fp63 {
            xo,
            frt,
            fra,
            frb,
            frc,
        } => fp::execute_fp63(state, xo, frt, fra, frb, frc),
        PpuInstruction::Fp59 {
            xo,
            frt,
            fra,
            frb,
            frc,
        } => fp::execute_fp59(state, xo, frt, fra, frb, frc),

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

// Vector (VMX / AltiVec) execution helpers live in exec_vec.rs.

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
    fn ldu_writes_ea_back_to_ra() {
        // ldu r7, -8(r4): read 8 bytes at r4-8, set r4 := r4-8.
        let mut s = PpuState::new();
        s.gpr[4] = 0x1020;
        let result = execute(
            &PpuInstruction::Ldu {
                rt: 7,
                ra: 4,
                imm: -8,
            },
            &mut s,
            uid(),
        );
        match result {
            PpuStepOutcome::Load { ea, size, rt } => {
                assert_eq!(ea, 0x1018);
                assert_eq!(size, 8);
                assert_eq!(rt, 7);
            }
            other => panic!("expected Load, got {:?}", other),
        }
        // Update form: RA holds the effective address after the instruction.
        assert_eq!(s.gpr[4], 0x1018);
    }

    #[test]
    fn rlwnm_rotates_by_rb_low_5_bits() {
        // rlwnm r0, r0, r8, 0, 31: full-word rotate left by r8 mod 32.
        let mut s = PpuState::new();
        s.gpr[0] = 0x0000_0000_1234_5678;
        s.gpr[8] = 8; // rotate by 8
        execute(
            &PpuInstruction::Rlwnm {
                ra: 0,
                rs: 0,
                rb: 8,
                mb: 0,
                me: 31,
            },
            &mut s,
            uid(),
        );
        // 0x12345678 rotated left by 8 = 0x34567812
        assert_eq!(s.gpr[0], 0x3456_7812);
    }

    #[test]
    fn rlwnm_ignores_high_bits_of_rb() {
        // Only low 5 bits of RB are used. 0x20 == 32 -> 0 rotation.
        let mut s = PpuState::new();
        s.gpr[1] = 0x0000_0000_DEAD_BEEF;
        s.gpr[2] = 0x20;
        execute(
            &PpuInstruction::Rlwnm {
                ra: 3,
                rs: 1,
                rb: 2,
                mb: 0,
                me: 31,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.gpr[3], 0xDEAD_BEEF);
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
    fn lha_returns_load_signed() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        let result = execute(
            &PpuInstruction::Lha {
                rt: 3,
                ra: 1,
                imm: 2,
            },
            &mut s,
            uid(),
        );
        match result {
            PpuStepOutcome::LoadSigned { ea, size, rt } => {
                assert_eq!(ea, 0x1002);
                assert_eq!(size, 2);
                assert_eq!(rt, 3);
            }
            other => panic!("expected LoadSigned, got {:?}", other),
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

    #[test]
    fn divdu_basic() {
        let mut s = PpuState::new();
        s.gpr[3] = 100;
        s.gpr[4] = 7;
        execute(
            &PpuInstruction::Divdu {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.gpr[5], 14);
    }

    #[test]
    fn divdu_divide_by_zero() {
        let mut s = PpuState::new();
        s.gpr[3] = 100;
        s.gpr[4] = 0;
        execute(
            &PpuInstruction::Divdu {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.gpr[5], 0);
    }

    #[test]
    fn divdu_large_values() {
        let mut s = PpuState::new();
        s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
        s.gpr[4] = 2;
        execute(
            &PpuInstruction::Divdu {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.gpr[5], 0x7FFF_FFFF_FFFF_FFFF);
    }

    #[test]
    fn divd_signed() {
        let mut s = PpuState::new();
        s.gpr[3] = (-100i64) as u64;
        s.gpr[4] = 7;
        execute(
            &PpuInstruction::Divd {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.gpr[5] as i64, -14);
    }

    #[test]
    fn divd_divide_by_zero() {
        let mut s = PpuState::new();
        s.gpr[3] = 100;
        s.gpr[4] = 0;
        execute(
            &PpuInstruction::Divd {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.gpr[5], 0);
    }

    #[test]
    fn mulld_basic() {
        let mut s = PpuState::new();
        s.gpr[3] = 7;
        s.gpr[4] = 8;
        execute(
            &PpuInstruction::Mulld {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.gpr[5], 56);
    }

    #[test]
    fn mulld_wraps_on_overflow() {
        let mut s = PpuState::new();
        s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
        s.gpr[4] = 2;
        execute(
            &PpuInstruction::Mulld {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            uid(),
        );
        // -1 * 2 = -2 (wrapping) = 0xFFFF_FFFF_FFFF_FFFE
        assert_eq!(s.gpr[5], 0xFFFF_FFFF_FFFF_FFFE);
    }

    #[test]
    fn adde_adds_with_carry_in_and_sets_carry_out() {
        let mut s = PpuState::new();
        // First: adde with CA=1 on overflowing low word produces carry-out.
        s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
        s.gpr[4] = 0;
        s.set_xer_ca(true);
        execute(
            &PpuInstruction::Adde {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            uid(),
        );
        // 0xFFFF... + 0 + 1 = 0, with carry.
        assert_eq!(s.gpr[5], 0);
        assert!(s.xer_ca());
    }

    #[test]
    fn adde_without_carry_clears_ca() {
        let mut s = PpuState::new();
        s.gpr[3] = 5;
        s.gpr[4] = 3;
        s.set_xer_ca(false);
        execute(
            &PpuInstruction::Adde {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.gpr[5], 8);
        assert!(!s.xer_ca());
    }

    #[test]
    fn mulhdu_takes_high_64_bits_of_u128_product() {
        // 0xFFFF_FFFF_FFFF_FFFF * 2 = 0x1_FFFF_FFFF_FFFF_FFFE,
        // high 64 bits = 1.
        let mut s = PpuState::new();
        s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
        s.gpr[4] = 2;
        execute(
            &PpuInstruction::Mulhdu {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.gpr[5], 1);
    }

    #[test]
    fn mulhdu_small_product_is_zero() {
        // 7 * 8 = 56; fits in 64 bits, so high 64 bits = 0.
        let mut s = PpuState::new();
        s.gpr[3] = 7;
        s.gpr[4] = 8;
        execute(
            &PpuInstruction::Mulhdu {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.gpr[5], 0);
    }

    #[test]
    fn ldarx_produces_load_like_ldx() {
        let mut s = PpuState::new();
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x8;
        let result = execute(
            &PpuInstruction::Ldarx {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            uid(),
        );
        match result {
            PpuStepOutcome::Load { ea, size, rt } => {
                assert_eq!(ea, 0x1008);
                assert_eq!(size, 8);
                assert_eq!(rt, 5);
            }
            other => panic!("expected Load, got {other:?}"),
        }
    }

    #[test]
    fn stdcx_always_succeeds_in_single_threaded() {
        let mut s = PpuState::new();
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x8;
        s.gpr[5] = 0xDEAD_BEEF_CAFE_BABE;
        let result = execute(
            &PpuInstruction::Stdcx {
                rs: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            uid(),
        );
        match result {
            PpuStepOutcome::Store { ea, size, value } => {
                assert_eq!(ea, 0x1008);
                assert_eq!(size, 8);
                assert_eq!(value, 0xDEAD_BEEF_CAFE_BABE);
            }
            other => panic!("expected Store, got {other:?}"),
        }
        // CR0 EQ must be set to indicate success.
        assert_eq!(s.cr_field(0), 0b0010);
    }
}
