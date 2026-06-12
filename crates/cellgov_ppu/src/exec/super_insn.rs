//! Predecoded shadow output: quickened single-instruction rewrites
//! and super-paired 2-instruction fusions. Profiling-driven; the
//! shadow build picks candidates above frequency thresholds and
//! rewrites them into these specialized variants. None of these
//! arms is ISA-native; they all decompose into one or two real PPC
//! instructions whose execution semantics they replicate.
//!
//! [Brunthaler2010 p:2 s:2] dispatch for quickened arms.
//! [ErtlGregg2003 p:20 s:6.3] dispatch for super-instruction arms.

use crate::exec::branch::branch_condition;
use crate::exec::memory_helpers::{buffer_store, load_ze};
use crate::exec::ExecuteVerdict;
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
            // [PPC-Book1 p:60 s:3.3.9] CR[4*BF .. 4*BF+3] <- c ||
            // XER[SO]. The SO bit is sticky and must be reflected in
            // every compare result.
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
        // [PPC-Book1 p:37 s:3.3.2 Load Word and Zero] lwz: RT <- 32 zeros || MEM(EA,4); EA = (RA|0)+EXTS(D).
        // [PPC-Book1 p:60 s:3.3.9 Compare Immediate] cmpi (cmpwi when L=0): signed 32-bit compare against EXTS(SI), CR field <- c || XER[SO].
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
        // [PPC-Book1 p:51 s:3.3.8 Add Immediate] addi (li when RA=0): RT <- EXTS(SI); the li mnemonic forces RA=0.
        // [PPC-Book1 p:42 s:3.3.3 Store Word] stw: MEM(EA,4) <- (RS)32:63; EA = (RA|0)+EXTS(D).
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
        // [PPC-Book1 p:81 s:3.3.13 Move From Special Purpose Register] mfspr (mflr when SPR=8): RS <- LR.
        // [PPC-Book1 p:42 s:3.3.3 Store Word] stw: MEM(EA,4) <- (RS)32:63; low 32 bits stored.
        PpuInstruction::MflrStw {
            rt,
            ra_store,
            store_offset,
        } => {
            state.gpr[rt as usize] = state.lr;
            let ea = state.ea_d_form(ra_store, store_offset);
            buffer_store(store_buf, state, ea, 4, state.gpr[rt as usize])
        }
        // [PPC-Book1 p:37 s:3.3.2 Load Word and Zero] lwz: RT <- 32 zeros || MEM(EA,4).
        // [PPC-Book1 p:81 s:3.3.13 Move To Special Purpose Register] mtspr (mtlr when SPR=8): LR <- (RS).
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
        // [PPC-Book1 p:81 s:3.3.13 Move From Special Purpose Register] mfspr (mflr when SPR=8): RS <- LR (full 64 bits).
        // [PPC-Book1 p:43 s:3.3.3 Store Doubleword] std: MEM(EA,8) <- (RS); EA = (RA|0)+EXTS(DS||0b00).
        PpuInstruction::MflrStd {
            rt,
            ra_store,
            store_offset,
        } => {
            state.gpr[rt as usize] = state.lr;
            let ea = state.ea_d_form(ra_store, store_offset);
            buffer_store(store_buf, state, ea, 8, state.gpr[rt as usize])
        }
        // [PPC-Book1 p:39 s:3.3.2 Load Doubleword] ld: RT <- MEM(EA,8); EA = (RA|0)+EXTS(DS||0b00).
        // [PPC-Book1 p:81 s:3.3.13 Move To Special Purpose Register] mtspr (mtlr when SPR=8): LR <- (RS).
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
        // [PPC-Book1 p:43 s:3.3.3 Store Doubleword] std (x2): MEM(EA,8) <- (RS) for each store; the second EA is constrained to EA1+8 by the fuser.
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
            if !v1.allows_writeback() {
                return v1;
            }
            let ea2 = ea1.wrapping_add(8);
            buffer_store(store_buf, state, ea2, 8, state.gpr[rs2 as usize])
        }
        // [PPC-Book1 p:60 s:3.3.9 Compare Immediate] cmpi (cmpwi when L=0): signed 32-bit compare against EXTS(SI), CR field <- c || XER[SO].
        // [PPC-Book1 p:18 s:2.3.1 Condition Register] CR field bits LT/GT/EQ/SO occupy positions 0/1/2/3 within the 4-bit field.
        // [PPC-Book1 p:24 s:2.4 Branch Conditional] bc: BO/BI gate the branch; target = CIA + EXTS(BD||0b00). Here CIA is the bc slot (super + 4).
        // [PPC-Book1 p:20 s:2.4.1 Branch Instructions Figure 21] BO field encodings: 0b001at = "branch if CR_BI = 0", 0b011at = "branch if CR_BI = 1", 0b1z1zz = "branch always".
        // [PPC-Book1 p:24 s:2.4 Branch Conditional B-form] BD displacement is sign-extended with 0b00 appended; the fused form pre-resolves AA=0 so target_offset is already EXTS(BD||0b00) bytes.
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
        // [PPC-Book1 p:60 s:3.3.9 Compare] cmp (cmpw when L=0): signed 32-bit register compare, CR field <- c || XER[SO].
        // [PPC-Book1 p:18 s:2.3.1 Condition Register] CR field bits LT/GT/EQ/SO occupy positions 0/1/2/3 within the 4-bit field.
        // [PPC-Book1 p:24 s:2.4 Branch Conditional] bc: BO/BI gate the branch; target = CIA + EXTS(BD||0b00).
        // [PPC-Book1 p:20 s:2.4.1 Branch Instructions Figure 21] BO field encodings: 0b001at = "branch if CR_BI = 0", 0b011at = "branch if CR_BI = 1"; non-CTR-decrementing forms ignore the CTR clause.
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
#[path = "tests/super_insn_tests.rs"]
mod tests;
