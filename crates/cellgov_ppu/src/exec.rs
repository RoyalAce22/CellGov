//! PPU instruction execution: mutates `PpuState` and stages memory
//! effects in response to a decoded `PpuInstruction`. Syscall
//! dispatch is delegated to the runtime via `ExecuteVerdict::Syscall`.
//!
//! The match dispatch in [`execute`] groups ISA variants by functional
//! unit and delegates each group to a peer module:
//!
//! - [`mem`]: integer / atomic / vector / floating-point loads and
//!   stores, plus `dcbz`.
//! - [`branch`]: `b`/`bc`/`bclr`/`bcctr` and the BO/BI condition
//!   evaluator they share.
//! - [`alu`]: arithmetic, logical, shift, rotate, compare, and
//!   CR/SPR moves.
//! - [`vec`]: VMX / AltiVec register-to-register arithmetic. The
//!   memory-touching vector ops (`lvx`, `lvlx`, `lvrx`, `stvx`)
//!   live in [`mem`] so all loads / stores share one
//!   store-buffer-forward / region-view path.
//! - [`super_insn`]: predecoded shadow output -- quickened single
//!   rewrites and super-paired 2-instruction fusions.
//! - [`fp`](crate::fp): double-precision floating-point arithmetic.
//!
//! The shared load / store vocabulary ([`load_ze`], [`load_se`],
//! [`buffer_store`], [`load_slice`]) lives here so cross-module
//! re-imports don't form dependency chains.

mod alu;
mod branch;
mod mem;
mod super_insn;
#[cfg(test)]
mod test_support;
mod vec;

use crate::fp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;
use cellgov_effects::Effect;
use cellgov_event::UnitId;

/// Outcome of a single `execute` call.
#[derive(Debug, PartialEq, Eq)]
pub enum ExecuteVerdict {
    /// Advance PC by 4.
    Continue,
    /// PC was written explicitly; caller must not advance.
    Branch,
    /// Yield to runtime syscall dispatch.
    Syscall,
    /// Architectural fault.
    Fault(PpuFault),
    /// Memory access at invalid effective address.
    MemFault(u64),
    /// Store buffer full; caller flushes, yields, then retries the
    /// same instruction.
    BufferFull,
}

/// PPU-specific fault categories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PpuFault {
    /// PC outside addressable memory.
    PcOutOfRange(u64),
    /// Invalid memory access address.
    InvalidAddress(u64),
    /// Unsupported syscall number.
    UnsupportedSyscall(u64),
    /// Decoded instruction had no execution arm (typically a VMX
    /// sub-opcode the dispatcher does not yet handle). The payload
    /// is the offending sub-opcode for diagnostics.
    UnimplementedInstruction(u64),
}

/// Linear search for a `[ea, ea+len)` slice covered entirely by one
/// region view.
#[inline]
pub(crate) fn load_slice<'a>(
    region_views: &[(u64, &'a [u8])],
    ea: u64,
    len: usize,
) -> Option<&'a [u8]> {
    let end = ea.checked_add(len as u64)?;
    for &(base, bytes) in region_views {
        let region_end = base + bytes.len() as u64;
        if ea >= base && end <= region_end {
            let offset = (ea - base) as usize;
            return Some(&bytes[offset..offset + len]);
        }
    }
    None
}

/// Zero-extending load; store buffer is checked first for forwarding.
#[inline]
pub(crate) fn load_ze(
    region_views: &[(u64, &[u8])],
    store_buf: &StoreBuffer,
    ea: u64,
    size: u8,
) -> Result<u64, u64> {
    if let Some(val) = store_buf.forward(ea, size) {
        return Ok(val as u64);
    }
    let slice = load_slice(region_views, ea, size as usize).ok_or(ea)?;
    Ok(match size {
        1 => slice[0] as u64,
        2 => u16::from_be_bytes([slice[0], slice[1]]) as u64,
        4 => u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]) as u64,
        8 => u64::from_be_bytes([
            slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
        ]),
        _ => {
            debug_assert!(false, "load_ze: unexpected size {size}");
            return Err(ea);
        }
    })
}

/// Sign-extending load; store buffer is checked first for forwarding.
#[inline]
pub(crate) fn load_se(
    region_views: &[(u64, &[u8])],
    store_buf: &StoreBuffer,
    ea: u64,
    size: u8,
) -> Result<u64, u64> {
    if let Some(val) = store_buf.forward(ea, size) {
        // Forward packs `size` bytes right-aligned into a u128.
        // Sign-extend from the size's MSB, not from u64 bit 63 --
        // the latter is always 0 for sub-8-byte forwarding and would
        // produce a positive result for a negative stored value.
        return Ok(match size {
            1 => (val as u8 as i8) as i64 as u64,
            2 => (val as u16 as i16) as i64 as u64,
            4 => (val as u32 as i32) as i64 as u64,
            8 => val as u64,
            _ => {
                debug_assert!(false, "load_se: unexpected size {size}");
                return Err(ea);
            }
        });
    }
    let slice = load_slice(region_views, ea, size as usize).ok_or(ea)?;
    Ok(match size {
        1 => (slice[0] as i8) as i64 as u64,
        2 => i16::from_be_bytes([slice[0], slice[1]]) as i64 as u64,
        4 => i32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]) as i64 as u64,
        // Identity for 64-bit; sign-extending a doubleword is a no-op
        // but the arm must exist so a future caller cannot fall to
        // the silent-zero default.
        8 => u64::from_be_bytes([
            slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
        ]),
        _ => {
            debug_assert!(false, "load_se: unexpected size {size}");
            return Err(ea);
        }
    })
}

/// Stage a store and drop any same-unit reservation that overlaps
/// the written byte range. Clearing must happen intra-step so a
/// subsequent `stwcx` on the same line observes the invalidation
/// without waiting for commit.
#[inline]
pub(crate) fn buffer_store(
    store_buf: &mut StoreBuffer,
    state: &mut PpuState,
    ea: u64,
    size: u8,
    value: u64,
) -> ExecuteVerdict {
    if let Some(line) = state.reservation {
        if line.overlaps_range(ea, size as u64) {
            state.reservation = None;
        }
    }
    if store_buf.insert(ea, size, value as u128) {
        ExecuteVerdict::Continue
    } else {
        ExecuteVerdict::BufferFull
    }
}

/// Execute one decoded PPU instruction.
///
/// Loads check the store buffer before falling back to `region_views`.
/// Stores stage into the buffer; the caller flushes at block boundaries.
pub fn execute(
    insn: &PpuInstruction,
    state: &mut PpuState,
    unit_id: UnitId,
    region_views: &[(u64, &[u8])],
    effects: &mut Vec<Effect>,
    store_buf: &mut StoreBuffer,
) -> ExecuteVerdict {
    match *insn {
        // Memory: integer / atomic / vector / FP loads and stores, plus dcbz.
        PpuInstruction::Lwz { .. }
        | PpuInstruction::Lbz { .. }
        | PpuInstruction::Lhz { .. }
        | PpuInstruction::Lha { .. }
        | PpuInstruction::Lwzu { .. }
        | PpuInstruction::Lbzu { .. }
        | PpuInstruction::Lhzu { .. }
        | PpuInstruction::Ldu { .. }
        | PpuInstruction::Ld { .. }
        | PpuInstruction::Lwa { .. }
        | PpuInstruction::Lwzx { .. }
        | PpuInstruction::Lbzx { .. }
        | PpuInstruction::Ldx { .. }
        | PpuInstruction::Lhzx { .. }
        | PpuInstruction::Stw { .. }
        | PpuInstruction::Stb { .. }
        | PpuInstruction::Stbu { .. }
        | PpuInstruction::Sth { .. }
        | PpuInstruction::Sthu { .. }
        | PpuInstruction::Std { .. }
        | PpuInstruction::Stwu { .. }
        | PpuInstruction::Stdu { .. }
        | PpuInstruction::Stwx { .. }
        | PpuInstruction::Stdx { .. }
        | PpuInstruction::Stdux { .. }
        | PpuInstruction::Stbx { .. }
        | PpuInstruction::Ldarx { .. }
        | PpuInstruction::Stdcx { .. }
        | PpuInstruction::Lwarx { .. }
        | PpuInstruction::Stwcx { .. }
        | PpuInstruction::Lvlx { .. }
        | PpuInstruction::Lvrx { .. }
        | PpuInstruction::Stvx { .. }
        | PpuInstruction::Lfs { .. }
        | PpuInstruction::Lfd { .. }
        | PpuInstruction::Stfs { .. }
        | PpuInstruction::Stfd { .. }
        | PpuInstruction::Stfsu { .. }
        | PpuInstruction::Stfdu { .. }
        | PpuInstruction::Stfiwx { .. }
        | PpuInstruction::Dcbz { .. } => {
            mem::execute(insn, state, unit_id, region_views, effects, store_buf)
        }

        // Branches.
        PpuInstruction::B { .. }
        | PpuInstruction::Bc { .. }
        | PpuInstruction::Bclr { .. }
        | PpuInstruction::Bcctr { .. } => branch::execute(insn, state),

        // Integer arithmetic / logical / shift / rotate / compare / CR-SPR moves.
        PpuInstruction::Addi { .. }
        | PpuInstruction::Addis { .. }
        | PpuInstruction::Subfic { .. }
        | PpuInstruction::Mulli { .. }
        | PpuInstruction::Addic { .. }
        | PpuInstruction::AddicDot { .. }
        | PpuInstruction::Add { .. }
        | PpuInstruction::Subf { .. }
        | PpuInstruction::Subfc { .. }
        | PpuInstruction::Subfe { .. }
        | PpuInstruction::Neg { .. }
        | PpuInstruction::Mullw { .. }
        | PpuInstruction::Mulhwu { .. }
        | PpuInstruction::Mulhw { .. }
        | PpuInstruction::Mulhdu { .. }
        | PpuInstruction::Mulhd { .. }
        | PpuInstruction::Adde { .. }
        | PpuInstruction::Addze { .. }
        | PpuInstruction::Divw { .. }
        | PpuInstruction::Divwu { .. }
        | PpuInstruction::Divd { .. }
        | PpuInstruction::Divdu { .. }
        | PpuInstruction::Mulld { .. }
        | PpuInstruction::Or { .. }
        | PpuInstruction::Orc { .. }
        | PpuInstruction::And { .. }
        | PpuInstruction::Nor { .. }
        | PpuInstruction::Andc { .. }
        | PpuInstruction::Xor { .. }
        | PpuInstruction::AndiDot { .. }
        | PpuInstruction::AndisDot { .. }
        | PpuInstruction::Slw { .. }
        | PpuInstruction::Srw { .. }
        | PpuInstruction::Srawi { .. }
        | PpuInstruction::Sraw { .. }
        | PpuInstruction::Srad { .. }
        | PpuInstruction::Sradi { .. }
        | PpuInstruction::Sld { .. }
        | PpuInstruction::Srd { .. }
        | PpuInstruction::Cntlzw { .. }
        | PpuInstruction::Cntlzd { .. }
        | PpuInstruction::Extsh { .. }
        | PpuInstruction::Extsb { .. }
        | PpuInstruction::Extsw { .. }
        | PpuInstruction::Ori { .. }
        | PpuInstruction::Oris { .. }
        | PpuInstruction::Xori { .. }
        | PpuInstruction::Xoris { .. }
        | PpuInstruction::Cmpwi { .. }
        | PpuInstruction::Cmplwi { .. }
        | PpuInstruction::Cmpdi { .. }
        | PpuInstruction::Cmpldi { .. }
        | PpuInstruction::Cmpw { .. }
        | PpuInstruction::Cmplw { .. }
        | PpuInstruction::Cmpd { .. }
        | PpuInstruction::Cmpld { .. }
        | PpuInstruction::Mftb { .. }
        | PpuInstruction::Mftbu { .. }
        | PpuInstruction::Mfcr { .. }
        | PpuInstruction::Mtcrf { .. }
        | PpuInstruction::Mflr { .. }
        | PpuInstruction::Mtlr { .. }
        | PpuInstruction::Mfctr { .. }
        | PpuInstruction::Mtctr { .. }
        | PpuInstruction::Rlwinm { .. }
        | PpuInstruction::Rlwimi { .. }
        | PpuInstruction::Rlwnm { .. }
        | PpuInstruction::Rldicl { .. }
        | PpuInstruction::Rldicr { .. }
        | PpuInstruction::Rldic { .. }
        | PpuInstruction::Rldimi { .. } => alu::execute(insn, state),

        // Vector arithmetic.
        PpuInstruction::Vxor { vt, va, vb } => {
            let va = state.vr[va as usize];
            let vb = state.vr[vb as usize];
            state.vr[vt as usize] = va ^ vb;
            ExecuteVerdict::Continue
        }
        // `lvx` rides the Vx encoding but is a memory load; route it
        // to the mem dispatcher so all vector loads share one
        // forwarding / region-view path.
        PpuInstruction::Vx {
            xo: 103,
            vt,
            va,
            vb,
        } => mem::execute_lvx(state, vt, va, vb, region_views, store_buf),
        PpuInstruction::Vx { xo, vt, va, vb } => vec::execute_vx(state, xo, vt, va, vb),
        PpuInstruction::Va { xo, vt, va, vb, vc } => vec::execute_va(state, xo, vt, va, vb, vc),
        PpuInstruction::Vsldoi { vt, va, vb, shb } => vec::execute_vsldoi(state, vt, va, vb, shb),

        // Floating-point arithmetic. TODO(fp-rc): `_rc` is preserved
        // at decode but record-form CR1 update is pending FPSCR
        // plumbing.
        PpuInstruction::Fp63 {
            xo,
            frt,
            fra,
            frb,
            frc,
            rc: _rc,
        } => fp::execute_fp63(state, xo, frt, fra, frb, frc),
        PpuInstruction::Fp59 {
            xo,
            frt,
            fra,
            frb,
            frc,
            rc: _rc,
        } => fp::execute_fp59(state, xo, frt, fra, frb, frc),

        // Predecoded shadow output: quickenings + super-pairs.
        PpuInstruction::Li { .. }
        | PpuInstruction::Mr { .. }
        | PpuInstruction::Slwi { .. }
        | PpuInstruction::Srwi { .. }
        | PpuInstruction::Clrlwi { .. }
        | PpuInstruction::Nop
        | PpuInstruction::CmpwZero { .. }
        | PpuInstruction::Clrldi { .. }
        | PpuInstruction::Sldi { .. }
        | PpuInstruction::Srdi { .. }
        | PpuInstruction::LwzCmpwi { .. }
        | PpuInstruction::LiStw { .. }
        | PpuInstruction::MflrStw { .. }
        | PpuInstruction::LwzMtlr { .. }
        | PpuInstruction::MflrStd { .. }
        | PpuInstruction::LdMtlr { .. }
        | PpuInstruction::StdStd { .. }
        | PpuInstruction::CmpwiBc { .. }
        | PpuInstruction::CmpwBc { .. }
        | PpuInstruction::Consumed => super_insn::execute(insn, state, region_views, store_buf),

        // TODO(sc-lev): `lev` is preserved at decode but not routed;
        // LV1 hypercall (LEV=1) dispatch lands here when it exists.
        // PS3 usermode always issues LEV=0.
        PpuInstruction::Sc { lev: _lev } => ExecuteVerdict::Syscall,
    }
}

#[cfg(test)]
#[path = "tests/exec_tests.rs"]
mod tests;
