//! Predecoded shadow output: quickened single-instruction rewrites
//! and super-paired 2-instruction fusions. The shadow build rewrites
//! a fixed set of patterns into these specialized variants; the set
//! was chosen offline from the frequency of the patterns in traced
//! titles. None of these arms is ISA-native; they all decompose into
//! one or two real PPC instructions whose execution semantics they
//! replicate.
//!
//! [Brunthaler2010 p:2 s:2] dispatch for quickened arms.
//! [ErtlGregg2003 p:20 s:6.3] dispatch for super-instruction arms.

use crate::exec::branch::branch_condition;
use crate::exec::mem::store_prechecked;
use crate::exec::memory_helpers::{buffer_store, load_ze, LoadPort, Width};
use crate::exec::ExecuteVerdict;
use crate::instruction::{branch_target, PpuInstruction};
use crate::state::{cmp_cr_field, PpuState};
use crate::store_buffer::StoreBuffer;
use cellgov_effects::Effect;
use cellgov_event::UnitId;

pub(crate) fn execute(
    insn: &PpuInstruction,
    state: &mut PpuState,
    unit_id: UnitId,
    region_views: &[cellgov_mem::RegionView<'_>],
    effects: &mut Vec<Effect>,
    store_buf: &mut StoreBuffer,
) -> ExecuteVerdict {
    match *insn {
        // Quickened (specialized) forms
        PpuInstruction::Li { rt, imm } => {
            state.set_gpr(rt as usize, imm as i64 as u64);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Mr { ra, rs } => {
            state.set_gpr(ra as usize, state.gpr[rs as usize]);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Slwi { ra, rs, n } => {
            let val = (state.gpr[rs as usize] as u32) << n;
            state.set_gpr(ra as usize, val as u64);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Srwi { ra, rs, n } => {
            let val = (state.gpr[rs as usize] as u32) >> n;
            state.set_gpr(ra as usize, val as u64);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Clrlwi { ra, rs, n } => {
            let mask = if n >= 32 { 0 } else { u32::MAX >> n };
            let val = (state.gpr[rs as usize] as u32) & mask;
            state.set_gpr(ra as usize, val as u64);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Nop => ExecuteVerdict::Continue,
        PpuInstruction::CmpwZero { bf, ra } => {
            // [PPC-Book1 p:60 s:3.3.9] CR[4*BF .. 4*BF+3] <- c ||
            // XER[SO]. The SO bit is sticky and must be reflected in
            // every compare result.
            cmpw(state, bf, state.gpr[ra as usize] as i32, 0);
            ExecuteVerdict::Continue
        }
        PpuInstruction::CmpdZero { bf, ra } => {
            // [PPC-Book1 p:60 s:3.3.9] cmpi with L = 1 compares all 64
            // bits of RA, signed.
            let a = state.gpr[ra as usize] as i64;
            state.set_cr_field(bf, cmp_cr_field(a < 0, a > 0, state.xer_so()));
            ExecuteVerdict::Continue
        }
        PpuInstruction::Clrldi { ra, rs, n } => {
            let mask = if n >= 64 { 0 } else { u64::MAX >> n };
            state.set_gpr(ra as usize, state.gpr[rs as usize] & mask);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Sldi { ra, rs, n } => {
            state.set_gpr(ra as usize, state.gpr[rs as usize] << n);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Srdi { ra, rs, n } => {
            state.set_gpr(ra as usize, state.gpr[rs as usize] >> n);
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:73 s:3.3.12] rlwinm: ROTL32 of the low word, ANDed with
        // MASK(MB+32, ME+32). With MB <= ME the mask holds no high-word bit.
        PpuInstruction::RlwinmMask {
            ra, rs, sh, mask, ..
        } => {
            let val = (state.gpr[rs as usize] as u32).rotate_left(u32::from(sh)) & mask;
            state.set_gpr(ra as usize, u64::from(val));
            ExecuteVerdict::Continue
        }
        PpuInstruction::Lis { rt, imm } => {
            state.set_gpr(rt as usize, (imm as i64 as u64) << 16);
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:20 s:2.4.1] BO 1a00t: decrement the whole CTR, then
        // branch if CTR != 0. The "a" and "t" bits do not change the result.
        PpuInstruction::Bdnz { offset, .. } => {
            state.set_ctr(state.ctr().wrapping_sub(1));
            branch_relative(state, state.ctr() != 0, offset)
        }
        // [PPC-Book1 p:20 s:2.4.1] BO 011at branches if CR bit BI is 1, and
        // BO 001at if it is 0. CTR does not change.
        PpuInstruction::BcTrue { bi, offset, .. } => {
            branch_relative(state, state.cr_bit(bi), offset)
        }
        PpuInstruction::BcFalse { bi, offset, .. } => {
            branch_relative(state, !state.cr_bit(bi), offset)
        }
        // [PPC-Book1 p:25 s:2.4] bclr: NIA <- LR[0:61] || 0b00. BO 1z1zz
        // branches always and leaves CTR alone.
        PpuInstruction::Blr { .. } => {
            state.pc = state.lr() & !3;
            ExecuteVerdict::Branch
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
            match load_ze(
                &mut LoadPort::new(region_views, store_buf, effects, unit_id),
                ea,
                Width::B4,
            ) {
                Ok(val) => {
                    state.set_gpr(rt as usize, val);
                    cmpw(state, bf, val as i32, cmp_imm as i32);
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
            state.set_gpr(rt as usize, val);
            let ea = state.ea_d_form(ra_store, store_offset);
            buffer_store(store_buf, region_views, ea, 4, val)
        }
        // [PPC-Book1 p:81 s:3.3.13 Move From Special Purpose Register] mfspr (mflr when SPR=8): RS <- LR.
        // [PPC-Book1 p:42 s:3.3.3 Store Word] stw: MEM(EA,4) <- (RS)32:63; low 32 bits stored.
        PpuInstruction::MflrStw {
            rt,
            ra_store,
            store_offset,
        } => {
            state.set_gpr(rt as usize, state.lr());
            let ea = state.ea_d_form(ra_store, store_offset);
            buffer_store(store_buf, region_views, ea, 4, state.gpr[rt as usize])
        }
        // [PPC-Book1 p:37 s:3.3.2 Load Word and Zero] lwz: RT <- 32 zeros || MEM(EA,4).
        // [PPC-Book1 p:81 s:3.3.13 Move To Special Purpose Register] mtspr (mtlr when SPR=8): LR <- (RS).
        PpuInstruction::LwzMtlr {
            rt,
            ra_load,
            offset,
        } => {
            let ea = state.ea_d_form(ra_load, offset);
            match load_ze(
                &mut LoadPort::new(region_views, store_buf, effects, unit_id),
                ea,
                Width::B4,
            ) {
                Ok(val) => {
                    state.set_gpr(rt as usize, val);
                    state.set_lr(val);
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
            state.set_gpr(rt as usize, state.lr());
            let ea = state.ea_d_form(ra_store, store_offset);
            buffer_store(store_buf, region_views, ea, 8, state.gpr[rt as usize])
        }
        // [PPC-Book1 p:39 s:3.3.2 Load Doubleword] ld: RT <- MEM(EA,8); EA = (RA|0)+EXTS(DS||0b00).
        // [PPC-Book1 p:81 s:3.3.13 Move To Special Purpose Register] mtspr (mtlr when SPR=8): LR <- (RS).
        PpuInstruction::LdMtlr {
            rt,
            ra_load,
            offset,
        } => {
            let ea = state.ea_d_form(ra_load, offset);
            match load_ze(
                &mut LoadPort::new(region_views, store_buf, effects, unit_id),
                ea,
                Width::B8,
            ) {
                Ok(val) => {
                    state.set_gpr(rt as usize, val);
                    state.set_lr(val);
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
            let ea2 = ea1.wrapping_add(8);
            let stores = [
                (ea1, 8u8, state.gpr[rs1 as usize]),
                (ea2, 8u8, state.gpr[rs2 as usize]),
            ];
            store_prechecked(store_buf, region_views, 2, stores.into_iter(), "stdstd")
        }
        // [PPC-Book1 p:39 s:3.3.2 Load Doubleword] ld (x2): RT <- MEM(EA,8); EA = (RA|0)+EXTS(DS||0b00).
        // [PPC-Book1 p:17 s:2.2] the second load runs after the first. The fuser keeps RT1 off a nonzero RA, so EA2 = EA1 + 8.
        PpuInstruction::LdLd {
            rt1,
            rt2,
            ra,
            offset1,
        } => {
            let ea1 = state.ea_d_form(ra, offset1);
            let mut port = LoadPort::new(region_views, store_buf, effects, unit_id);
            let first = match load_ze(&mut port, ea1, Width::B8) {
                Ok(val) => val,
                Err(ea) => return ExecuteVerdict::MemFault(ea),
            };
            state.set_gpr(rt1 as usize, first);
            match load_ze(&mut port, ea1.wrapping_add(8), Width::B8) {
                Ok(val) => {
                    state.set_gpr(rt2 as usize, val);
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        // [PPC-Book1 p:34 s:3.3.2 Load Byte and Zero with Update] lbzu: RT <- 56 zeros || MEM(EA,1); RA <- EA; EA = (RA)+EXTS(D).
        // [PPC-Book1 p:60 s:3.3.9 Compare Immediate] cmpwi: signed 32-bit compare against 0, read after the load wrote RT and RA.
        PpuInstruction::LbzuCmpwi {
            rt,
            ra,
            offset,
            bf,
            cmp_ra,
        } => {
            let ea = state.ea_d_form(ra, offset);
            match load_ze(
                &mut LoadPort::new(region_views, store_buf, effects, unit_id),
                ea,
                Width::B1,
            ) {
                Ok(val) => {
                    state.set_gpr(rt as usize, val);
                    state.set_gpr(ra as usize, ea);
                    cmpw(state, bf, state.gpr[cmp_ra as usize] as i32, 0);
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        // [PPC-Book1 p:67 s:3.3.11 OR] or: RA <- (RS) | (RB).
        // [PPC-Book1 p:52 s:3.3.8 Add] add: RT <- (RA) + (RB), read after the or wrote its RA.
        PpuInstruction::OrAdd {
            ra,
            rs,
            rb,
            rt,
            add_ra,
            add_rb,
        } => {
            state.set_gpr(ra as usize, state.gpr[rs as usize] | state.gpr[rb as usize]);
            let sum = state.gpr[add_ra as usize].wrapping_add(state.gpr[add_rb as usize]);
            state.set_gpr(rt as usize, sum);
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:51 s:3.3.8 Add Immediate Shifted] addis (lis when RA=0): RT <- EXTS(SI || 0x0000).
        // [PPC-Book1 p:66 s:3.3.11 OR Immediate] ori: RA <- (RS) | (48 zeros || UI), so the word is hi || lo, sign-extended.
        PpuInstruction::LisOri { rt, value } => {
            state.set_gpr(rt as usize, value as i64 as u64);
            ExecuteVerdict::Continue
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
            cmpw(state, bf, state.gpr[ra as usize] as i32, imm as i32);
            take_bc(state, bo, bi, target_offset)
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
            cmpw(
                state,
                bf,
                state.gpr[ra as usize] as i32,
                state.gpr[rb as usize] as i32,
            );
            take_bc(state, bo, bi, target_offset)
        }
        // [PPC-Book1 p:60 s:3.3.9] cmpi with L = 1 (cmpdi): signed compare of
        // all 64 bits of RA against EXTS(SI); then the bc slot as above.
        PpuInstruction::CmpdiBc {
            bf,
            ra,
            imm,
            bo,
            bi,
            target_offset,
        } => {
            cmpw(state, bf, state.gpr[ra as usize] as i64, i64::from(imm));
            take_bc(state, bo, bi, target_offset)
        }
        // [PPC-Book1 p:60 s:3.3.9] cmp with L = 1 (cmpd): signed 64-bit
        // register compare; then the bc slot as above.
        PpuInstruction::CmpdBc {
            bf,
            ra,
            rb,
            bo,
            bi,
            target_offset,
        } => {
            cmpw(
                state,
                bf,
                state.gpr[ra as usize] as i64,
                state.gpr[rb as usize] as i64,
            );
            take_bc(state, bo, bi, target_offset)
        }
        // [PPC-Book1 p:61 s:3.3.9] cmpl with L = 0 (cmplw): unsigned compare
        // of bits 32:63 of RA and RB; then the bc slot as above.
        PpuInstruction::CmplwBc {
            bf,
            ra,
            rb,
            bo,
            bi,
            target_offset,
        } => {
            cmpw(
                state,
                bf,
                state.gpr[ra as usize] as u32,
                state.gpr[rb as usize] as u32,
            );
            take_bc(state, bo, bi, target_offset)
        }
        PpuInstruction::Consumed => {
            // The fetch loop runs the stored word at a Consumed slot, so
            // the placeholder reaches no execution arm.
            debug_assert!(false, "a Consumed placeholder reached execute");
            ExecuteVerdict::Fault(crate::exec::PpuFault::Internal(".consumed"))
        }

        _ => unreachable!("super_insn::execute called with non-super variant"),
    }
}

/// Compare of `a` with `b` into CR field `bf`, with SO from XER.
///
/// `T` sets the width and signedness of the compare.
#[inline]
fn cmpw<T: Ord>(state: &mut PpuState, bf: u8, a: T, b: T) {
    state.set_cr_field(bf, cmp_cr_field(a < b, a > b, state.xer_so()));
}

/// A quickened `bc` with AA = 0: branch to `offset` bytes past this
/// instruction when `taken`.
#[inline]
fn branch_relative(state: &mut PpuState, taken: bool, offset: i16) -> ExecuteVerdict {
    if taken {
        state.pc = branch_target(state.pc, i32::from(offset), false);
        ExecuteVerdict::Branch
    } else {
        ExecuteVerdict::Continue
    }
}

/// The fused bc slot: branch to `target_offset` bytes past the bc slot
/// (super + 4) when BO/BI hold.
#[inline]
fn take_bc(state: &mut PpuState, bo: u8, bi: u8, target_offset: i16) -> ExecuteVerdict {
    if branch_condition(state, bo, bi) {
        state.pc = (state.pc.wrapping_add(4) as i64).wrapping_add(target_offset as i64) as u64;
        ExecuteVerdict::Branch
    } else {
        ExecuteVerdict::Continue
    }
}

#[cfg(test)]
#[path = "tests/super_insn_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/consumed_tests.rs"]
mod consumed_tests;
