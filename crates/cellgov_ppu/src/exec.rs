//! PPU instruction execution: mutates `PpuState` and stages memory
//! effects in response to a decoded `PpuInstruction`. Syscall
//! dispatch is delegated to the runtime via `ExecuteVerdict::Syscall`.

use crate::fp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_sync::ReservedLine;
use cellgov_time::GuestTicks;

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
fn load_ze(
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
        _ => 0,
    })
}

/// Sign-extending load; store buffer is checked first for forwarding.
#[inline]
fn load_se(
    region_views: &[(u64, &[u8])],
    store_buf: &StoreBuffer,
    ea: u64,
    size: u8,
) -> Result<u64, u64> {
    if let Some(val) = store_buf.forward(ea, size) {
        return Ok(val as i64 as u64);
    }
    let slice = load_slice(region_views, ea, size as usize).ok_or(ea)?;
    Ok(match size {
        1 => (slice[0] as i8) as i64 as u64,
        2 => i16::from_be_bytes([slice[0], slice[1]]) as i64 as u64,
        4 => i32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]) as i64 as u64,
        _ => 0,
    })
}

/// Stage a store and drop any same-unit reservation that overlaps
/// the written byte range. Clearing must happen intra-step so a
/// subsequent `stwcx` on the same line observes the invalidation
/// without waiting for commit.
#[inline]
fn buffer_store(
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
/// Stores stage into the buffer; the caller flushes to `effects` at
/// block or step boundaries.
pub fn execute(
    insn: &PpuInstruction,
    state: &mut PpuState,
    unit_id: UnitId,
    region_views: &[(u64, &[u8])],
    effects: &mut Vec<Effect>,
    store_buf: &mut StoreBuffer,
) -> ExecuteVerdict {
    match *insn {
        // Integer loads
        PpuInstruction::Lwz { rt, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            match load_ze(region_views, store_buf, ea, 4) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Lbz { rt, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            match load_ze(region_views, store_buf, ea, 1) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Lhz { rt, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            match load_ze(region_views, store_buf, ea, 2) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Lha { rt, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            match load_se(region_views, store_buf, ea, 2) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Lwzu { rt, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            match load_ze(region_views, store_buf, ea, 4) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    state.gpr[ra as usize] = ea;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Lbzu { rt, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            match load_ze(region_views, store_buf, ea, 1) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    state.gpr[ra as usize] = ea;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Lhzu { rt, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            match load_ze(region_views, store_buf, ea, 2) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    state.gpr[ra as usize] = ea;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Ldu { rt, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            match load_ze(region_views, store_buf, ea, 8) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    state.gpr[ra as usize] = ea;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Ld { rt, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            match load_ze(region_views, store_buf, ea, 8) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Lwzx { rt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            match load_ze(region_views, store_buf, ea, 4) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Lbzx { rt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            match load_ze(region_views, store_buf, ea, 1) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Ldx { rt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            match load_ze(region_views, store_buf, ea, 8) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Lhzx { rt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            match load_ze(region_views, store_buf, ea, 2) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }

        // Integer stores
        PpuInstruction::Stw { rs, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            buffer_store(store_buf, state, ea, 4, state.gpr[rs as usize])
        }
        PpuInstruction::Stb { rs, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            buffer_store(store_buf, state, ea, 1, state.gpr[rs as usize])
        }
        PpuInstruction::Sth { rs, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            buffer_store(store_buf, state, ea, 2, state.gpr[rs as usize])
        }
        PpuInstruction::Std { rs, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            buffer_store(store_buf, state, ea, 8, state.gpr[rs as usize])
        }
        PpuInstruction::Stwu { rs, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            let v = buffer_store(store_buf, state, ea, 4, state.gpr[rs as usize]);
            if v == ExecuteVerdict::Continue {
                state.gpr[ra as usize] = ea;
            }
            v
        }
        PpuInstruction::Stdu { rs, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            let v = buffer_store(store_buf, state, ea, 8, state.gpr[rs as usize]);
            if v == ExecuteVerdict::Continue {
                state.gpr[ra as usize] = ea;
            }
            v
        }
        PpuInstruction::Stwx { rs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            buffer_store(store_buf, state, ea, 4, state.gpr[rs as usize])
        }
        PpuInstruction::Stdx { rs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            buffer_store(store_buf, state, ea, 8, state.gpr[rs as usize])
        }
        PpuInstruction::Stdux { rs, ra, rb } => {
            let ea = state.gpr[ra as usize].wrapping_add(state.gpr[rb as usize]);
            let verdict = buffer_store(store_buf, state, ea, 8, state.gpr[rs as usize]);
            if matches!(verdict, ExecuteVerdict::Continue) {
                state.gpr[ra as usize] = ea;
            }
            verdict
        }
        PpuInstruction::Ldarx { rt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            match load_ze(region_views, store_buf, ea, 8) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    let line = ReservedLine::containing(ea);
                    state.reservation = Some(line);
                    effects.push(Effect::ReservationAcquire {
                        line_addr: line.addr(),
                        source: unit_id,
                    });
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Stdcx { rs, ra, rb } => {
            // Local reservation alone is authoritative here:
            // cross-unit invalidation is applied at step start by
            // the context refresh, and same-unit overlap is cleared
            // in `buffer_store` for every preceding store.
            let ea = state.ea_x_form(ra, rb);
            let success = match state.reservation {
                Some(line) => line.addr() == ReservedLine::containing(ea).addr(),
                None => false,
            };
            if success {
                state.set_cr_field(0, 0b0010);
                let range = match ByteRange::new(GuestAddr::new(ea), 8) {
                    Some(r) => r,
                    None => return ExecuteVerdict::MemFault(ea),
                };
                let value = state.gpr[rs as usize];
                let bytes = value.to_be_bytes();
                // Flush plain stores first so ConditionalStore
                // follows them in program order.
                store_buf.flush(effects, unit_id);
                effects.push(Effect::ConditionalStore {
                    range,
                    bytes: WritePayload::from_slice(&bytes),
                    ordering: PriorityClass::Normal,
                    source: unit_id,
                    source_time: GuestTicks::ZERO,
                });
                // Forwarding-only entry: a subsequent lwarx in the
                // same step sees the bytes this stwcx committed
                // rather than pre-step memory. The flush pass skips
                // conditional entries so no SharedWriteIntent fires.
                store_buf.insert_conditional(ea, 8, value as u128);
            } else {
                state.set_cr_field(0, 0b0000);
            }
            state.reservation = None;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Lwarx { rt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            match load_ze(region_views, store_buf, ea, 4) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    let line = ReservedLine::containing(ea);
                    state.reservation = Some(line);
                    effects.push(Effect::ReservationAcquire {
                        line_addr: line.addr(),
                        source: unit_id,
                    });
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Stwcx { rs, ra, rb } => {
            // See `Stdcx` for the reservation / flush / forward-entry
            // contract.
            let ea = state.ea_x_form(ra, rb);
            let success = match state.reservation {
                Some(line) => line.addr() == ReservedLine::containing(ea).addr(),
                None => false,
            };
            if success {
                state.set_cr_field(0, 0b0010);
                let range = match ByteRange::new(GuestAddr::new(ea), 4) {
                    Some(r) => r,
                    None => return ExecuteVerdict::MemFault(ea),
                };
                let value32 = state.gpr[rs as usize] as u32;
                let bytes = value32.to_be_bytes();
                store_buf.flush(effects, unit_id);
                effects.push(Effect::ConditionalStore {
                    range,
                    bytes: WritePayload::from_slice(&bytes),
                    ordering: PriorityClass::Normal,
                    source: unit_id,
                    source_time: GuestTicks::ZERO,
                });
                store_buf.insert_conditional(ea, 4, value32 as u128);
            } else {
                state.set_cr_field(0, 0b0000);
            }
            state.reservation = None;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Stbx { rs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            buffer_store(store_buf, state, ea, 1, state.gpr[rs as usize])
        }

        // Integer arithmetic / logical
        PpuInstruction::Addi { rt, ra, imm } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            state.gpr[rt as usize] = base.wrapping_add(imm as i64 as u64);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Addis { rt, ra, imm } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            state.gpr[rt as usize] = base.wrapping_add((imm as i64 as u64) << 16);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Subfic { rt, ra, imm } => {
            let a = state.gpr[ra as usize];
            let b = imm as i64 as u64;
            state.gpr[rt as usize] = b.wrapping_sub(a);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Mulli { rt, ra, imm } => {
            let a = state.gpr[ra as usize] as i64;
            let b = imm as i64;
            state.gpr[rt as usize] = a.wrapping_mul(b) as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Addic { rt, ra, imm } => {
            let a = state.gpr[ra as usize];
            let b = imm as i64 as u64;
            state.gpr[rt as usize] = a.wrapping_add(b);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Add { rt, ra, rb } => {
            state.gpr[rt as usize] = state.gpr[ra as usize].wrapping_add(state.gpr[rb as usize]);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Subf { rt, ra, rb } => {
            state.gpr[rt as usize] = state.gpr[rb as usize].wrapping_sub(state.gpr[ra as usize]);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Subfc { rt, ra, rb } => {
            let a = state.gpr[ra as usize];
            let b = state.gpr[rb as usize];
            let (result, borrow) = b.overflowing_sub(a);
            state.gpr[rt as usize] = result;
            state.set_xer_ca(!borrow);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Subfe { rt, ra, rb } => {
            let a = state.gpr[ra as usize];
            let b = state.gpr[rb as usize];
            let ca_in: u64 = state.xer_ca() as u64;
            let (s1, c1) = b.overflowing_add(!a);
            let (s2, c2) = s1.overflowing_add(ca_in);
            state.gpr[rt as usize] = s2;
            state.set_xer_ca(c1 || c2);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Neg { rt, ra } => {
            state.gpr[rt as usize] = (state.gpr[ra as usize] as i64).wrapping_neg() as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Mullw { rt, ra, rb } => {
            let a = state.gpr[ra as usize] as i32;
            let b = state.gpr[rb as usize] as i32;
            state.gpr[rt as usize] = (a as i64).wrapping_mul(b as i64) as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Mulhwu { rt, ra, rb } => {
            let a = state.gpr[ra as usize] as u32 as u64;
            let b = state.gpr[rb as usize] as u32 as u64;
            state.gpr[rt as usize] = (a * b) >> 32;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Mulhw { rt, ra, rb } => {
            let a = state.gpr[ra as usize] as i32 as i64;
            let b = state.gpr[rb as usize] as i32 as i64;
            // Signed 32x32 -> high 32, sign-extended to 64.
            state.gpr[rt as usize] = ((a * b) >> 32) as i32 as i64 as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Mulhdu { rt, ra, rb } => {
            let a = state.gpr[ra as usize] as u128;
            let b = state.gpr[rb as usize] as u128;
            state.gpr[rt as usize] = ((a * b) >> 64) as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Mulhd { rt, ra, rb } => {
            let a = state.gpr[ra as usize] as i64 as i128;
            let b = state.gpr[rb as usize] as i64 as i128;
            state.gpr[rt as usize] = ((a * b) >> 64) as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Adde { rt, ra, rb } => {
            let a = state.gpr[ra as usize];
            let b = state.gpr[rb as usize];
            let ca_in: u64 = state.xer_ca() as u64;
            let (sum1, c1) = a.overflowing_add(b);
            let (sum2, c2) = sum1.overflowing_add(ca_in);
            state.gpr[rt as usize] = sum2;
            state.set_xer_ca(c1 || c2);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Addze { rt, ra } => {
            // adde with RB = 0: only one overflow edge.
            let a = state.gpr[ra as usize];
            let ca_in: u64 = state.xer_ca() as u64;
            let (sum, c) = a.overflowing_add(ca_in);
            state.gpr[rt as usize] = sum;
            state.set_xer_ca(c);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Divw { rt, ra, rb } => {
            let a = state.gpr[ra as usize] as i32;
            let b = state.gpr[rb as usize] as i32;
            let result = if b == 0 { 0 } else { a.wrapping_div(b) };
            state.gpr[rt as usize] = result as i64 as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Divwu { rt, ra, rb } => {
            let a = state.gpr[ra as usize] as u32;
            let b = state.gpr[rb as usize] as u32;
            let result = a.checked_div(b).unwrap_or(0);
            state.gpr[rt as usize] = result as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Divd { rt, ra, rb } => {
            let a = state.gpr[ra as usize] as i64;
            let b = state.gpr[rb as usize] as i64;
            let result = if b == 0 { 0 } else { a.wrapping_div(b) };
            state.gpr[rt as usize] = result as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Divdu { rt, ra, rb } => {
            let a = state.gpr[ra as usize];
            let b = state.gpr[rb as usize];
            let result = a.checked_div(b).unwrap_or(0);
            state.gpr[rt as usize] = result;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Mulld { rt, ra, rb } => {
            let a = state.gpr[ra as usize];
            let b = state.gpr[rb as usize];
            state.gpr[rt as usize] = a.wrapping_mul(b);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Or { ra, rs, rb } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] | state.gpr[rb as usize];
            ExecuteVerdict::Continue
        }
        PpuInstruction::Orc { ra, rs, rb } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] | !state.gpr[rb as usize];
            ExecuteVerdict::Continue
        }
        PpuInstruction::And { ra, rs, rb } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] & state.gpr[rb as usize];
            ExecuteVerdict::Continue
        }
        PpuInstruction::Nor { ra, rs, rb } => {
            state.gpr[ra as usize] = !(state.gpr[rs as usize] | state.gpr[rb as usize]);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Andc { ra, rs, rb } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] & !state.gpr[rb as usize];
            ExecuteVerdict::Continue
        }
        PpuInstruction::Xor { ra, rs, rb } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] ^ state.gpr[rb as usize];
            ExecuteVerdict::Continue
        }
        PpuInstruction::AndiDot { ra, rs, imm } => {
            let result = state.gpr[rs as usize] & imm as u64;
            state.gpr[ra as usize] = result;
            let cr_val = if (result as i64) < 0 {
                0b1000
            } else if result > 0 {
                0b0100
            } else {
                0b0010
            };
            state.set_cr_field(0, cr_val);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Slw { ra, rs, rb } => {
            let shift = state.gpr[rb as usize] & 0x3F;
            let val = state.gpr[rs as usize] as u32;
            let result = if shift < 32 { val << shift } else { 0 };
            state.gpr[ra as usize] = result as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Srw { ra, rs, rb } => {
            let shift = state.gpr[rb as usize] & 0x3F;
            let val = state.gpr[rs as usize] as u32;
            let result = if shift < 32 { val >> shift } else { 0 };
            state.gpr[ra as usize] = result as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Srawi { ra, rs, sh } => {
            let val = state.gpr[rs as usize] as i32;
            let result = val >> sh;
            let ca = val < 0 && (val as u32) << (32 - sh) != 0;
            state.gpr[ra as usize] = result as i64 as u64;
            state.set_xer_ca(ca);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Sraw { ra, rs, rb } => {
            let shift = state.gpr[rb as usize] & 0x3F;
            let val = state.gpr[rs as usize] as i32;
            if shift < 32 {
                let result = val >> shift;
                let ca = val < 0 && (val as u32) << (32 - shift as u32) != 0;
                state.gpr[ra as usize] = result as i64 as u64;
                state.set_xer_ca(ca);
            } else {
                let result = val >> 31;
                state.gpr[ra as usize] = result as i64 as u64;
                state.set_xer_ca(val < 0);
            }
            ExecuteVerdict::Continue
        }
        PpuInstruction::Srad { ra, rs, rb } => {
            let shift = state.gpr[rb as usize] & 0x7F;
            let val = state.gpr[rs as usize] as i64;
            if shift < 64 {
                let result = val >> shift;
                let ca = val < 0 && (val as u64) << (64 - shift) != 0;
                state.gpr[ra as usize] = result as u64;
                state.set_xer_ca(ca);
            } else {
                let result = val >> 63;
                state.gpr[ra as usize] = result as u64;
                state.set_xer_ca(val < 0);
            }
            ExecuteVerdict::Continue
        }
        PpuInstruction::Sradi { ra, rs, sh } => {
            let shift = sh as u64;
            let val = state.gpr[rs as usize] as i64;
            let result = val >> shift;
            let ca = val < 0 && shift > 0 && (val as u64) << (64 - shift) != 0;
            state.gpr[ra as usize] = result as u64;
            state.set_xer_ca(ca);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Sld { ra, rs, rb } => {
            let shift = state.gpr[rb as usize] & 0x7F;
            let result = if shift < 64 {
                state.gpr[rs as usize] << shift
            } else {
                0
            };
            state.gpr[ra as usize] = result;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Srd { ra, rs, rb } => {
            let shift = state.gpr[rb as usize] & 0x7F;
            let result = if shift < 64 {
                state.gpr[rs as usize] >> shift
            } else {
                0
            };
            state.gpr[ra as usize] = result;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Cntlzw { ra, rs } => {
            let val = state.gpr[rs as usize] as u32;
            state.gpr[ra as usize] = val.leading_zeros() as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Cntlzd { ra, rs } => {
            state.gpr[ra as usize] = state.gpr[rs as usize].leading_zeros() as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Extsh { ra, rs } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] as i16 as i64 as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Extsb { ra, rs } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] as i8 as i64 as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Extsw { ra, rs } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] as i32 as i64 as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Ori { ra, rs, imm } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] | imm as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Oris { ra, rs, imm } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] | ((imm as u64) << 16);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Xori { ra, rs, imm } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] ^ imm as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Xoris { ra, rs, imm } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] ^ ((imm as u64) << 16);
            ExecuteVerdict::Continue
        }

        // Compare
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
            ExecuteVerdict::Continue
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
            ExecuteVerdict::Continue
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
            ExecuteVerdict::Continue
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
            ExecuteVerdict::Continue
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
            ExecuteVerdict::Continue
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
            ExecuteVerdict::Continue
        }

        // Branch
        PpuInstruction::B { offset, aa, link } => {
            if link {
                state.lr = state.pc + 4;
            }
            if aa {
                state.pc = (offset as u64) & 0xFFFF_FFFF_FFFF_FFFC;
            } else {
                state.pc = (state.pc as i64).wrapping_add(offset as i64) as u64;
            }
            ExecuteVerdict::Branch
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
                ExecuteVerdict::Branch
            } else {
                ExecuteVerdict::Continue
            }
        }
        PpuInstruction::Bclr { bo, bi, link } => {
            let target = state.lr & !3;
            if link {
                state.lr = state.pc + 4;
            }
            if branch_condition(state, bo, bi) {
                state.pc = target;
                ExecuteVerdict::Branch
            } else {
                ExecuteVerdict::Continue
            }
        }
        PpuInstruction::Bcctr { bo, bi, link } => {
            if link {
                state.lr = state.pc + 4;
            }
            if branch_condition(state, bo, bi) {
                state.pc = state.ctr & !3;
                ExecuteVerdict::Branch
            } else {
                ExecuteVerdict::Continue
            }
        }

        // Special-purpose register moves
        PpuInstruction::Mftb { rt } => {
            // Deterministic per-read advance; host wall time is
            // not observable.
            state.tb += 512;
            state.gpr[rt as usize] = state.tb;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Mfcr { rt } => {
            state.gpr[rt as usize] = state.cr as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Mtcrf { rs, crm } => {
            // Each bit in CRM selects a 4-bit CR field.
            let val = (state.gpr[rs as usize] >> 32) as u32;
            for i in 0..8u8 {
                if crm & (1 << (7 - i)) != 0 {
                    let shift = (7 - i) * 4;
                    let field_bits = (val >> shift) & 0xF;
                    let mask = 0xF << shift;
                    state.cr = (state.cr & !mask) | (field_bits << shift);
                }
            }
            ExecuteVerdict::Continue
        }
        PpuInstruction::Mflr { rt } => {
            state.gpr[rt as usize] = state.lr;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Mtlr { rs } => {
            state.lr = state.gpr[rs as usize];
            ExecuteVerdict::Continue
        }
        PpuInstruction::Mfctr { rt } => {
            state.gpr[rt as usize] = state.ctr;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Mtctr { rs } => {
            state.ctr = state.gpr[rs as usize];
            ExecuteVerdict::Continue
        }

        // Rotate / mask
        PpuInstruction::Rlwinm { ra, rs, sh, mb, me } => {
            let val = state.gpr[rs as usize] as u32;
            let rotated = val.rotate_left(sh as u32);
            let mask = rlwinm_mask(mb, me);
            state.gpr[ra as usize] = (rotated & mask) as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Rlwimi { ra, rs, sh, mb, me } => {
            let val = state.gpr[rs as usize] as u32;
            let rotated = val.rotate_left(sh as u32);
            let mask = rlwinm_mask(mb, me);
            let prior = state.gpr[ra as usize] as u32;
            let merged = (rotated & mask) | (prior & !mask);
            state.gpr[ra as usize] = merged as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Rlwnm { ra, rs, rb, mb, me } => {
            let val = state.gpr[rs as usize] as u32;
            let n = (state.gpr[rb as usize] & 0x1F) as u32;
            let rotated = val.rotate_left(n);
            let mask = rlwinm_mask(mb, me);
            state.gpr[ra as usize] = (rotated & mask) as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Rldicl { ra, rs, sh, mb } => {
            let rotated = state.gpr[rs as usize].rotate_left(sh as u32);
            state.gpr[ra as usize] = rotated & mask64(mb, 63);
            ExecuteVerdict::Continue
        }
        PpuInstruction::Rldicr { ra, rs, sh, me } => {
            let rotated = state.gpr[rs as usize].rotate_left(sh as u32);
            state.gpr[ra as usize] = rotated & mask64(0, me);
            ExecuteVerdict::Continue
        }

        // Vector (AltiVec / VMX)
        PpuInstruction::Vxor { vt, va, vb } => {
            state.vr[vt as usize] = state.vr[va as usize] ^ state.vr[vb as usize];
            ExecuteVerdict::Continue
        }
        PpuInstruction::Lvlx { vt, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let addr = base.wrapping_add(state.gpr[rb as usize]);
            let aligned = addr & !15u64;
            let val = if let Some(v) = store_buf.forward(aligned, 16) {
                v
            } else {
                let slice = match load_slice(region_views, aligned, 16) {
                    Some(s) => s,
                    None => return ExecuteVerdict::MemFault(aligned),
                };
                let mut bytes = [0u8; 16];
                bytes.copy_from_slice(slice);
                u128::from_be_bytes(bytes)
            };
            let shift = ((addr & 15) * 8) as u32;
            state.vr[vt as usize] = if shift == 0 { val } else { val << shift };
            ExecuteVerdict::Continue
        }
        PpuInstruction::Lvrx { vt, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let addr = base.wrapping_add(state.gpr[rb as usize]);
            let aligned = addr & !15u64;
            let val = if let Some(v) = store_buf.forward(aligned, 16) {
                v
            } else {
                let slice = match load_slice(region_views, aligned, 16) {
                    Some(s) => s,
                    None => return ExecuteVerdict::MemFault(aligned),
                };
                let mut bytes = [0u8; 16];
                bytes.copy_from_slice(slice);
                u128::from_be_bytes(bytes)
            };
            let lo = addr & 15;
            state.vr[vt as usize] = if lo == 0 {
                0
            } else {
                val >> ((16 - lo) * 8) as u32
            };
            ExecuteVerdict::Continue
        }
        PpuInstruction::Vx { xo, vt, va, vb } => {
            crate::exec_vec::execute_vx(state, xo, vt, va, vb, region_views, store_buf)
        }
        PpuInstruction::Va { xo, vt, va, vb, vc } => {
            crate::exec_vec::execute_va(state, xo, vt, va, vb, vc)
        }
        PpuInstruction::Stvx { vs, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]) & !15u64;
            if store_buf.insert(ea, 16, state.vr[vs as usize]) {
                ExecuteVerdict::Continue
            } else {
                ExecuteVerdict::BufferFull
            }
        }

        // Floating-point loads/stores
        PpuInstruction::Lfs { frt, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            match load_ze(region_views, store_buf, ea, 4) {
                Ok(bits) => {
                    state.fpr[frt as usize] = (f32::from_bits(bits as u32) as f64).to_bits();
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Lfd { frt, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            match load_ze(region_views, store_buf, ea, 8) {
                Ok(bits) => {
                    state.fpr[frt as usize] = bits;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Stfs { frs, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            let f = f64::from_bits(state.fpr[frs as usize]) as f32;
            let val = f.to_bits() as u128;
            if store_buf.insert(ea, 4, val) {
                ExecuteVerdict::Continue
            } else {
                ExecuteVerdict::BufferFull
            }
        }
        PpuInstruction::Stfd { frs, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            let val = state.fpr[frs as usize] as u128;
            if store_buf.insert(ea, 8, val) {
                ExecuteVerdict::Continue
            } else {
                ExecuteVerdict::BufferFull
            }
        }
        PpuInstruction::Stfsu { frs, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            let f = f64::from_bits(state.fpr[frs as usize]) as f32;
            let val = f.to_bits() as u128;
            if store_buf.insert(ea, 4, val) {
                state.gpr[ra as usize] = ea;
                ExecuteVerdict::Continue
            } else {
                ExecuteVerdict::BufferFull
            }
        }
        PpuInstruction::Stfdu { frs, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            let val = state.fpr[frs as usize] as u128;
            if store_buf.insert(ea, 8, val) {
                state.gpr[ra as usize] = ea;
                ExecuteVerdict::Continue
            } else {
                ExecuteVerdict::BufferFull
            }
        }
        // Unlike stfs, stfiwx stores the low 32 FPR bits verbatim
        // (no round-convert to single precision).
        PpuInstruction::Stfiwx { frs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            buffer_store(
                store_buf,
                state,
                ea,
                4,
                state.fpr[frs as usize] & 0xFFFF_FFFF,
            )
        }

        // Floating-point arithmetic (opcode 63, double precision)
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
            state.set_cr_field(bf, cr_val);
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
                    state.set_cr_field(bf, cr_val);
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
            state.set_cr_field(bf, cr_val);
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
            state.set_cr_field(bf, cr_val);
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

        PpuInstruction::Sc => ExecuteVerdict::Syscall,
    }
}

/// Evaluate a PPC BO/BI branch condition. Decrements CTR as a side
/// effect when BO bit 0x04 is clear.
///
/// BO bits (MSB->LSB): 0x10 skip CR test, 0x08 CR polarity,
/// 0x04 skip CTR decrement, 0x02 CTR-zero polarity, 0x01 hint.
fn branch_condition(state: &mut PpuState, bo: u8, bi: u8) -> bool {
    let decr_ctr = (bo & 0x04) == 0;
    if decr_ctr {
        state.ctr = state.ctr.wrapping_sub(1);
    }

    let ctr_ok = (bo & 0x04) != 0 || ((state.ctr != 0) ^ ((bo & 0x02) != 0));
    let cr_ok = (bo & 0x10) != 0 || (state.cr_bit(bi) == ((bo & 0x08) != 0));

    ctr_ok && cr_ok
}

/// 32-bit rlwinm mask. `mb > me` wraps to bits `[0..me]` and `[mb..31]`.
fn rlwinm_mask(mb: u8, me: u8) -> u32 {
    if mb <= me {
        let top = 0xFFFF_FFFFu32 >> mb;
        let bottom = 0xFFFF_FFFFu32 << (31 - me);
        top & bottom
    } else {
        let top = 0xFFFF_FFFFu32 << (31 - me);
        let bottom = 0xFFFF_FFFFu32 >> mb;
        top | bottom
    }
}

/// 64-bit PPC mask from MSB-numbered bits `mb..=me`; `mb > me` wraps
/// to `[0..me]` and `[mb..63]`.
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
#[path = "tests/exec_tests.rs"]
mod tests;
