//! PPU instruction dispatch: decodes [`PpuInstruction`] variants to
//! per-unit submodules, mutating [`PpuState`] and staging memory
//! [`Effect`]s. Syscalls escape via [`ExecuteVerdict::Syscall`].
//!
//! Memory-touching vector ops (`lvx`, `lvlx`, `lvrx`, `stvx`) route
//! through `mem` rather than `vec` so every load / store shares
//! one store-buffer-forward / region-view path.

mod alu;
mod branch;
mod cr;
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
// [PPC-Book1 p:5 s:1.5] non-branching insns set NIA=CIA+4; branches assign NIA explicitly.
#[derive(Debug, PartialEq, Eq)]
pub enum ExecuteVerdict {
    /// Advance PC by 4.
    Continue,
    /// PC was written explicitly; caller must not advance.
    Branch,
    /// Yield to runtime syscall dispatch.
    // [PPC-Book3 p:12 s:2.3.1] sc with LEV=1 invokes the hypervisor; LEV>1 reserved.
    // [PPC-Book1 p:26 s:2.4.2] sc LEV-field encoding (instruction bits 20:25 reserved).
    Syscall {
        /// LEV field of `sc`: 0 = kernel syscall, 1 = hypercall,
        /// greater than 1 reserved.
        lev: u8,
    },
    /// Architectural fault.
    Fault(PpuFault),
    /// Memory access into an unmapped region. Carries the
    /// `cellgov_mem::MemError` produced at the loader boundary; the
    /// effective address is reachable via
    /// `MemError::Unmapped(FaultContext).addr`.
    MemFault(cellgov_mem::MemError),
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
    /// Decoded instruction had no execution arm; payload is the
    /// offending sub-opcode.
    UnimplementedInstruction(u64),
}

impl std::fmt::Display for PpuFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PcOutOfRange(pc) => write!(f, "PPU PC out of range at 0x{pc:016x}"),
            Self::InvalidAddress(addr) => {
                write!(f, "PPU invalid address at 0x{addr:016x}")
            }
            Self::UnsupportedSyscall(nr) => {
                write!(f, "PPU unsupported syscall {nr}")
            }
            Self::UnimplementedInstruction(op) => {
                write!(f, "PPU unimplemented instruction sub-opcode 0x{op:x}")
            }
        }
    }
}

impl std::error::Error for PpuFault {}

/// Linear search for `[ea, ea+len)` covered by one region view.
///
/// O(n) over `region_views`; n is small (single-digit) per dispatch.
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

/// Synthesize a `MemError::Unmapped` for `ea` with no nearest-region
/// labels populated; the helper does not have a `GuestMemory`
/// reference to walk, only the flat region-view slice.
#[inline]
fn unmapped(ea: u64) -> cellgov_mem::MemError {
    cellgov_mem::MemError::Unmapped(cellgov_mem::FaultContext {
        addr: ea,
        nearest_below: None,
        nearest_above: None,
    })
}

/// Zero-extending load with store-buffer forwarding.
///
/// Slow path overlays buffered stores onto the region view, so
/// multi-store stitching (eight `stb`s read as one `ld`) and partial
/// overlaps with pre-block memory both resolve correctly.
#[inline]
pub(crate) fn load_ze(
    region_views: &[(u64, &[u8])],
    store_buf: &StoreBuffer,
    ea: u64,
    size: u8,
) -> Result<u64, cellgov_mem::MemError> {
    if let Some(val) = store_buf.forward(ea, size) {
        return Ok(val as u64);
    }
    let slice = load_slice(region_views, ea, size as usize).ok_or_else(|| unmapped(ea))?;
    let mut bytes = [0u8; 8];
    let n = size as usize;
    bytes[..n].copy_from_slice(&slice[..n]);
    store_buf.overlay_range(ea, &mut bytes[..n]);
    Ok(match size {
        1 => bytes[0] as u64,
        2 => u16::from_be_bytes([bytes[0], bytes[1]]) as u64,
        4 => u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as u64,
        8 => u64::from_be_bytes(bytes),
        _ => {
            debug_assert!(false, "load_ze: unexpected size {size}");
            return Err(unmapped(ea));
        }
    })
}

/// Sign-extending load with store-buffer forwarding. See [`load_ze`].
#[inline]
pub(crate) fn load_se(
    region_views: &[(u64, &[u8])],
    store_buf: &StoreBuffer,
    ea: u64,
    size: u8,
) -> Result<u64, cellgov_mem::MemError> {
    if let Some(val) = store_buf.forward(ea, size) {
        // `forward` right-aligns `size` bytes; sign must come from
        // the size's MSB, not u64 bit 63 (always 0 for sub-doubleword).
        return Ok(match size {
            1 => (val as u8 as i8) as i64 as u64,
            2 => (val as u16 as i16) as i64 as u64,
            4 => (val as u32 as i32) as i64 as u64,
            8 => val as u64,
            _ => {
                debug_assert!(false, "load_se: unexpected size {size}");
                return Err(unmapped(ea));
            }
        });
    }
    let slice = load_slice(region_views, ea, size as usize).ok_or_else(|| unmapped(ea))?;
    let mut bytes = [0u8; 8];
    let n = size as usize;
    bytes[..n].copy_from_slice(&slice[..n]);
    store_buf.overlay_range(ea, &mut bytes[..n]);
    Ok(match size {
        1 => (bytes[0] as i8) as i64 as u64,
        2 => i16::from_be_bytes([bytes[0], bytes[1]]) as i64 as u64,
        4 => i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as i64 as u64,
        8 => u64::from_be_bytes(bytes),
        _ => {
            debug_assert!(false, "load_se: unexpected size {size}");
            return Err(unmapped(ea));
        }
    })
}

/// Stage a store and drop any same-unit reservation overlapping
/// the written range.
///
/// Reservation clearing is intra-step so a later `stwcx` in the
/// same block observes the invalidation pre-commit.
// [PPC-Book2 p:10 s:1.7.3.1] reservation lost when any store hits the reservation granule.
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
/// Caller flushes `store_buf` at block boundaries; on
/// [`ExecuteVerdict::BufferFull`] the same instruction must be
/// retried after a flush.
pub fn execute(
    insn: &PpuInstruction,
    state: &mut PpuState,
    unit_id: UnitId,
    region_views: &[(u64, &[u8])],
    effects: &mut Vec<Effect>,
    store_buf: &mut StoreBuffer,
) -> ExecuteVerdict {
    match *insn {
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
        | PpuInstruction::Lfsx { .. }
        | PpuInstruction::Lfsux { .. }
        | PpuInstruction::Lfdx { .. }
        | PpuInstruction::Lfdux { .. }
        | PpuInstruction::Stfsx { .. }
        | PpuInstruction::Stfsux { .. }
        | PpuInstruction::Stfdx { .. }
        | PpuInstruction::Stfdux { .. }
        | PpuInstruction::Dcbz { .. } => {
            mem::execute(insn, state, unit_id, region_views, effects, store_buf)
        }

        PpuInstruction::B { .. }
        | PpuInstruction::Bc { .. }
        | PpuInstruction::Bclr { .. }
        | PpuInstruction::Bcctr { .. } => branch::execute(insn, state),

        // CR-logical (XL-form opcode 19).
        PpuInstruction::Mcrf { .. }
        | PpuInstruction::Crand { .. }
        | PpuInstruction::Crandc { .. }
        | PpuInstruction::Cror { .. }
        | PpuInstruction::Crorc { .. }
        | PpuInstruction::Crxor { .. }
        | PpuInstruction::Crnand { .. }
        | PpuInstruction::Crnor { .. }
        | PpuInstruction::Creqv { .. } => cr::execute(insn, state),

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

        PpuInstruction::Vxor { vt, va, vb } => {
            let va = state.vr[va as usize];
            let vb = state.vr[vb as usize];
            state.vr[vt as usize] = va ^ vb;
            ExecuteVerdict::Continue
        }
        // `lvx` (Vx xo=103) is a memory load encoded under the Vx
        // form; route to mem so vector loads share the forwarding path.
        // [AltiVec-PEM p:6-21] lvx EA = (rA|0)+rB masked with ~0xF; loads 16 bytes.
        PpuInstruction::Vx {
            xo: 103,
            vt,
            va,
            vb,
        } => mem::execute_lvx(state, vt, va, vb, region_views, store_buf),
        PpuInstruction::Vx { xo, vt, va, vb } => vec::execute_vx(state, xo, vt, va, vb),
        PpuInstruction::Va { xo, vt, va, vb, vc } => vec::execute_va(state, xo, vt, va, vb, vc),
        PpuInstruction::Vsldoi { vt, va, vb, shb } => vec::execute_vsldoi(state, vt, va, vb, shb),

        // TODO(fp-rc): record-form CR1 update pending FPSCR plumbing;
        // `_rc` is preserved at decode.
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

        // [PPC-Book1 p:26 s:2.4.2] sc surfaces LEV to the system; service dispatch is runtime-defined.
        PpuInstruction::Sc { lev } => ExecuteVerdict::Syscall { lev },
    }
}

#[cfg(test)]
#[path = "../tests/exec_tests.rs"]
mod tests;
