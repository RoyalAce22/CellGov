//! PPU instruction execution.
//!
//! Takes a decoded `PpuInstruction` and a `PpuState`, applies the
//! instruction's semantics (register mutation, inlined loads/stores),
//! and returns an `ExecuteVerdict`. Syscalls emit
//! `ExecuteVerdict::Syscall`; actual dispatch lives in
//! `lib.rs::run_until_yield` and `syscall.rs`.

use crate::fp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;
use cellgov_effects::Effect;
use cellgov_event::UnitId;

/// What happened after executing one instruction.
#[derive(Debug, PartialEq, Eq)]
pub enum ExecuteVerdict {
    /// Instruction executed, advance PC by 4.
    Continue,
    /// PC was set explicitly (branch taken). Do not advance PC.
    Branch,
    /// Syscall: yield for runtime dispatch.
    Syscall,
    /// Instruction caused an architecture fault.
    Fault(PpuFault),
    /// Memory access at an invalid address. The u64 is the faulting EA.
    MemFault(u64),
    /// Store buffer capacity exceeded. The outer loop should flush the
    /// buffer, yield BudgetExhausted, and retry the instruction.
    BufferFull,
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

/// Look up a byte slice in the region views table.
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

/// Zero-extending load from guest memory, checking the store buffer
/// first for forwarding.
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

/// Sign-extending load from guest memory, checking the store buffer
/// first for forwarding.
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

/// Insert a store into the store buffer. Returns `BufferFull` if the
/// buffer has no room; returns `Continue` on success.
#[inline]
fn buffer_store(store_buf: &mut StoreBuffer, ea: u64, size: u8, value: u64) -> ExecuteVerdict {
    if store_buf.insert(ea, size, value as u128) {
        ExecuteVerdict::Continue
    } else {
        ExecuteVerdict::BufferFull
    }
}

/// Execute a single decoded PPU instruction against the given state.
///
/// Loads check the store buffer first for forwarding, then fall back
/// to `region_views`. Stores insert into the store buffer; the
/// caller flushes the buffer to effects at block/step boundaries.
pub fn execute(
    insn: &PpuInstruction,
    state: &mut PpuState,
    _unit_id: UnitId,
    region_views: &[(u64, &[u8])],
    _effects: &mut Vec<Effect>,
    store_buf: &mut StoreBuffer,
) -> ExecuteVerdict {
    match *insn {
        // =================================================================
        // Integer loads (inlined: read from region_views)
        // =================================================================
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
        // Indexed loads
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

        // =================================================================
        // Integer stores (buffered: insert into store buffer)
        // =================================================================
        PpuInstruction::Stw { rs, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            buffer_store(store_buf, ea, 4, state.gpr[rs as usize])
        }
        PpuInstruction::Stb { rs, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            buffer_store(store_buf, ea, 1, state.gpr[rs as usize])
        }
        PpuInstruction::Sth { rs, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            buffer_store(store_buf, ea, 2, state.gpr[rs as usize])
        }
        PpuInstruction::Std { rs, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            buffer_store(store_buf, ea, 8, state.gpr[rs as usize])
        }
        PpuInstruction::Stwu { rs, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            let v = buffer_store(store_buf, ea, 4, state.gpr[rs as usize]);
            if v == ExecuteVerdict::Continue {
                state.gpr[ra as usize] = ea;
            }
            v
        }
        PpuInstruction::Stdu { rs, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            let v = buffer_store(store_buf, ea, 8, state.gpr[rs as usize]);
            if v == ExecuteVerdict::Continue {
                state.gpr[ra as usize] = ea;
            }
            v
        }
        // Indexed stores
        PpuInstruction::Stwx { rs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            buffer_store(store_buf, ea, 4, state.gpr[rs as usize])
        }
        PpuInstruction::Stdx { rs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            buffer_store(store_buf, ea, 8, state.gpr[rs as usize])
        }
        PpuInstruction::Ldarx { rt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            match load_ze(region_views, store_buf, ea, 8) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Stdcx { rs, ra, rb } => {
            // Single-threaded: reservation never lost, CAS always succeeds.
            state.set_cr_field(0, 0b0010); // EQ
            let ea = state.ea_x_form(ra, rb);
            buffer_store(store_buf, ea, 8, state.gpr[rs as usize])
        }
        PpuInstruction::Lwarx { rt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            match load_ze(region_views, store_buf, ea, 4) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Stwcx { rs, ra, rb } => {
            // Single-threaded: reservation never lost, CAS always succeeds.
            state.set_cr_field(0, 0b0010); // EQ
            let ea = state.ea_x_form(ra, rb);
            buffer_store(store_buf, ea, 4, state.gpr[rs as usize])
        }
        PpuInstruction::Stbx { rs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            buffer_store(store_buf, ea, 1, state.gpr[rs as usize])
        }

        // =================================================================
        // Integer arithmetic / logical
        // =================================================================
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
            // CA bit would be set here but we don't track XER yet
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
            // Signed 32x32 -> high 32 bits, sign-extended to 64-bit.
            let a = state.gpr[ra as usize] as i32 as i64;
            let b = state.gpr[rb as usize] as i32 as i64;
            state.gpr[rt as usize] = ((a * b) >> 32) as i32 as i64 as u64;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Mulhdu { rt, ra, rb } => {
            let a = state.gpr[ra as usize] as u128;
            let b = state.gpr[rb as usize] as u128;
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
            // Equivalent to adde with RB = 0, so only one overflow
            // edge matters (ra overflowing when ca_in = 1).
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
            // andi. always updates CR0
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
            state.gpr[ra as usize] = result as i64 as u64;
            // CA bit would be set here but we don't track XER yet
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

        // =================================================================
        // Branch
        // =================================================================
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

        // =================================================================
        // Special-purpose register moves
        // =================================================================
        PpuInstruction::Mftb { rt } => {
            state.tb += 512; // advance deterministically per read
            state.gpr[rt as usize] = state.tb;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Mfcr { rt } => {
            state.gpr[rt as usize] = state.cr as u64;
            ExecuteVerdict::Continue
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

        // =================================================================
        // Rotate / mask
        // =================================================================
        PpuInstruction::Rlwinm { ra, rs, sh, mb, me } => {
            let val = state.gpr[rs as usize] as u32;
            let rotated = val.rotate_left(sh as u32);
            let mask = rlwinm_mask(mb, me);
            state.gpr[ra as usize] = (rotated & mask) as u64;
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

        // =================================================================
        // Vector (AltiVec / VMX)
        // =================================================================
        PpuInstruction::Vxor { vt, va, vb } => {
            state.vr[vt as usize] = state.vr[va as usize] ^ state.vr[vb as usize];
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

        // =================================================================
        // System call
        // =================================================================
        // =================================================================
        // Floating-point loads/stores
        // =================================================================
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
        // stfiwx: store the low 32 bits of the FPR as an integer word
        // at the x-form effective address. Unlike stfs it does NOT
        // round-convert to single precision; the bits go out verbatim.
        PpuInstruction::Stfiwx { frs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            buffer_store(store_buf, ea, 4, state.fpr[frs as usize] & 0xFFFF_FFFF)
        }

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

        // =================================================================
        // Quickened (specialized) forms
        // =================================================================
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

        // =================================================================
        // Superinstructions (compound 2-instruction pairs)
        // =================================================================
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
            buffer_store(store_buf, ea, 4, val)
        }
        PpuInstruction::MflrStw {
            rt,
            ra_store,
            store_offset,
        } => {
            state.gpr[rt as usize] = state.lr;
            let ea = state.ea_d_form(ra_store, store_offset);
            buffer_store(store_buf, ea, 4, state.gpr[rt as usize])
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
        PpuInstruction::Consumed => {
            unreachable!("Consumed slots should be skipped by the fetch loop")
        }

        PpuInstruction::Sc => ExecuteVerdict::Syscall,
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

    /// Shorthand: execute with no memory regions. Good for ALU /
    /// branch / SPR tests that never touch memory.
    fn exec_no_mem(insn: &PpuInstruction, s: &mut PpuState) -> ExecuteVerdict {
        let mut effects = Vec::new();
        let mut store_buf = crate::store_buffer::StoreBuffer::new();
        execute(insn, s, uid(), &[], &mut effects, &mut store_buf)
    }

    /// Execute with a single flat memory region starting at `base`.
    /// After execution, flushes the store buffer into `effects`.
    fn exec_with_mem(
        insn: &PpuInstruction,
        s: &mut PpuState,
        base: u64,
        mem: &[u8],
        effects: &mut Vec<Effect>,
    ) -> ExecuteVerdict {
        let views: [(u64, &[u8]); 1] = [(base, mem)];
        let mut store_buf = crate::store_buffer::StoreBuffer::new();
        let v = execute(insn, s, uid(), &views, effects, &mut store_buf);
        store_buf.flush(effects, uid());
        v
    }

    #[test]
    fn addi_with_ra_zero_is_li() {
        let mut s = PpuState::new();
        exec_no_mem(
            &PpuInstruction::Addi {
                rt: 3,
                ra: 0,
                imm: 42,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 42);
    }

    #[test]
    fn addi_with_ra_nonzero_adds() {
        let mut s = PpuState::new();
        s.gpr[5] = 100;
        exec_no_mem(
            &PpuInstruction::Addi {
                rt: 3,
                ra: 5,
                imm: -10,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 90);
    }

    #[test]
    fn addis_shifts_left_16() {
        let mut s = PpuState::new();
        exec_no_mem(
            &PpuInstruction::Addis {
                rt: 3,
                ra: 0,
                imm: 1,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 0x10000);
    }

    #[test]
    fn ori_zero_is_move() {
        let mut s = PpuState::new();
        s.gpr[5] = 0xCAFE;
        exec_no_mem(
            &PpuInstruction::Ori {
                ra: 3,
                rs: 5,
                imm: 0,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 0xCAFE);
    }

    #[test]
    fn cmpwi_sets_cr_field() {
        let mut s = PpuState::new();
        s.gpr[3] = 10;
        exec_no_mem(
            &PpuInstruction::Cmpwi {
                bf: 0,
                ra: 3,
                imm: 10,
            },
            &mut s,
        );
        assert_eq!(s.cr_field(0), 0b0010); // EQ
    }

    #[test]
    fn branch_unconditional() {
        let mut s = PpuState::new();
        s.pc = 0x1000;
        let result = exec_no_mem(
            &PpuInstruction::B {
                offset: -8,
                aa: false,
                link: false,
            },
            &mut s,
        );
        assert!(matches!(result, ExecuteVerdict::Branch));
        assert_eq!(s.pc, 0x0FF8);
    }

    #[test]
    fn bl_sets_lr() {
        let mut s = PpuState::new();
        s.pc = 0x1000;
        exec_no_mem(
            &PpuInstruction::B {
                offset: 0x100,
                aa: false,
                link: true,
            },
            &mut s,
        );
        assert_eq!(s.lr, 0x1004);
        assert_eq!(s.pc, 0x1100);
    }

    #[test]
    fn ba_branches_to_absolute_address() {
        let mut s = PpuState::new();
        s.pc = 0x2000;
        let result = exec_no_mem(
            &PpuInstruction::B {
                offset: 0x100,
                aa: true,
                link: false,
            },
            &mut s,
        );
        assert!(matches!(result, ExecuteVerdict::Branch));
        assert_eq!(
            s.pc, 0x100,
            "aa=true: target is offset itself, not PC+offset"
        );
    }

    #[test]
    fn bla_sets_lr_and_branches_absolute() {
        let mut s = PpuState::new();
        s.pc = 0x2000;
        exec_no_mem(
            &PpuInstruction::B {
                offset: 0x400,
                aa: true,
                link: true,
            },
            &mut s,
        );
        assert_eq!(s.lr, 0x2004);
        assert_eq!(s.pc, 0x400);
    }

    #[test]
    fn blr_returns_to_lr() {
        let mut s = PpuState::new();
        s.pc = 0x2000;
        s.lr = 0x1000;
        // BO=0x14 = always taken (don't test CR, don't decr CTR)
        let result = exec_no_mem(
            &PpuInstruction::Bclr {
                bo: 0x14,
                bi: 0,
                link: false,
            },
            &mut s,
        );
        assert!(matches!(result, ExecuteVerdict::Branch));
        assert_eq!(s.pc, 0x1000);
    }

    #[test]
    fn mflr_mtlr_roundtrip() {
        let mut s = PpuState::new();
        s.gpr[5] = 0xABCD;
        exec_no_mem(&PpuInstruction::Mtlr { rs: 5 }, &mut s);
        assert_eq!(s.lr, 0xABCD);
        exec_no_mem(&PpuInstruction::Mflr { rt: 3 }, &mut s);
        assert_eq!(s.gpr[3], 0xABCD);
    }

    #[test]
    fn rlwinm_slwi() {
        let mut s = PpuState::new();
        s.gpr[5] = 0x0001;
        // slwi r3, r5, 16 = rlwinm r3, r5, 16, 0, 15
        exec_no_mem(
            &PpuInstruction::Rlwinm {
                ra: 3,
                rs: 5,
                sh: 16,
                mb: 0,
                me: 15,
            },
            &mut s,
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
        // Place 8 bytes of data at address 0x1018.
        let mut mem = vec![0u8; 0x1028];
        mem[0x1018..0x1020].copy_from_slice(&0xDEAD_BEEF_CAFE_BABEu64.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[4] = 0x1020;
        let mut effects = Vec::new();
        let result = exec_with_mem(
            &PpuInstruction::Ldu {
                rt: 7,
                ra: 4,
                imm: -8,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(result, ExecuteVerdict::Continue);
        assert_eq!(s.gpr[7], 0xDEAD_BEEF_CAFE_BABE);
        // Update form: RA holds the effective address after the instruction.
        assert_eq!(s.gpr[4], 0x1018);
    }

    #[test]
    fn rlwnm_rotates_by_rb_low_5_bits() {
        // rlwnm r0, r0, r8, 0, 31: full-word rotate left by r8 mod 32.
        let mut s = PpuState::new();
        s.gpr[0] = 0x0000_0000_1234_5678;
        s.gpr[8] = 8; // rotate by 8
        exec_no_mem(
            &PpuInstruction::Rlwnm {
                ra: 0,
                rs: 0,
                rb: 8,
                mb: 0,
                me: 31,
            },
            &mut s,
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
        exec_no_mem(
            &PpuInstruction::Rlwnm {
                ra: 3,
                rs: 1,
                rb: 2,
                mb: 0,
                me: 31,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 0xDEAD_BEEF);
    }

    #[test]
    fn vxor_self_zeros_vector_register() {
        let mut s = PpuState::new();
        s.vr[5] = 0xDEAD_BEEF_DEAD_BEEF_DEAD_BEEF_DEAD_BEEFu128;
        exec_no_mem(
            &PpuInstruction::Vxor {
                vt: 5,
                va: 5,
                vb: 5,
            },
            &mut s,
        );
        assert_eq!(s.vr[5], 0);
    }

    #[test]
    fn stvx_aligns_ea_and_emits_store_effect() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        s.gpr[8] = 0x1F;
        s.vr[0] = 0xAABB_CCDD_EEFF_0011_2233_4455_6677_8899u128;
        let mut effects = Vec::new();
        let result = exec_with_mem(
            &PpuInstruction::Stvx {
                vs: 0,
                ra: 1,
                rb: 8,
            },
            &mut s,
            0,
            &[0u8; 0x2000],
            &mut effects,
        );
        assert_eq!(result, ExecuteVerdict::Continue);
        // Should have emitted one SharedWriteIntent at aligned EA 0x1010.
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::SharedWriteIntent { range, .. } => {
                assert_eq!(range.start().raw(), 0x1010);
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn extsw_sign_extends_low_32_bits() {
        let mut s = PpuState::new();
        s.gpr[3] = 0x0000_0000_8000_0000; // bit 31 set in low word
        exec_no_mem(&PpuInstruction::Extsw { ra: 4, rs: 3 }, &mut s);
        assert_eq!(s.gpr[4], 0xFFFF_FFFF_8000_0000);
    }

    #[test]
    fn sc_returns_syscall() {
        let mut s = PpuState::new();
        let result = exec_no_mem(&PpuInstruction::Sc, &mut s);
        assert!(matches!(result, ExecuteVerdict::Syscall));
    }

    #[test]
    fn lwz_loads_from_memory() {
        // Place 0xDEADBEEF at address 0x1008.
        let mut mem = vec![0u8; 0x2000];
        mem[0x1008..0x100C].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        let mut effects = Vec::new();
        let result = exec_with_mem(
            &PpuInstruction::Lwz {
                rt: 3,
                ra: 1,
                imm: 8,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(result, ExecuteVerdict::Continue);
        assert_eq!(s.gpr[3], 0xDEAD_BEEF);
    }

    #[test]
    fn lwz_mem_fault_on_bad_address() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        let result = exec_no_mem(
            &PpuInstruction::Lwz {
                rt: 3,
                ra: 1,
                imm: 8,
            },
            &mut s,
        );
        assert_eq!(result, ExecuteVerdict::MemFault(0x1008));
    }

    #[test]
    fn lha_sign_extends_halfword() {
        // Place 0xFF80 (-128 as i16) at address 0x1002.
        let mut mem = vec![0u8; 0x2000];
        mem[0x1002..0x1004].copy_from_slice(&0xFF80u16.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        let mut effects = Vec::new();
        let result = exec_with_mem(
            &PpuInstruction::Lha {
                rt: 3,
                ra: 1,
                imm: 2,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(result, ExecuteVerdict::Continue);
        assert_eq!(s.gpr[3] as i64, -128);
    }

    #[test]
    fn stw_emits_store_effect() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        s.gpr[5] = 0xDEADBEEF;
        let mut effects = Vec::new();
        let result = exec_with_mem(
            &PpuInstruction::Stw {
                rs: 5,
                ra: 1,
                imm: 0,
            },
            &mut s,
            0,
            &[0u8; 0x2000],
            &mut effects,
        );
        assert_eq!(result, ExecuteVerdict::Continue);
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x1000);
                assert_eq!(bytes.bytes(), &0xDEAD_BEEFu32.to_be_bytes());
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn bc_beq_taken() {
        let mut s = PpuState::new();
        s.pc = 0x1000;
        s.set_cr_field(0, 0b0010); // EQ set
                                   // beq cr0, +8: BO=0x0C (test CR, don't decr CTR), BI=2 (EQ bit of cr0)
        let result = exec_no_mem(
            &PpuInstruction::Bc {
                bo: 0x0C,
                bi: 2,
                offset: 8,
                link: false,
            },
            &mut s,
        );
        assert!(matches!(result, ExecuteVerdict::Branch));
        assert_eq!(s.pc, 0x1008);
    }

    #[test]
    fn bc_beq_not_taken() {
        let mut s = PpuState::new();
        s.pc = 0x1000;
        s.set_cr_field(0, 0b0100); // GT set, not EQ
        let result = exec_no_mem(
            &PpuInstruction::Bc {
                bo: 0x0C,
                bi: 2,
                offset: 8,
                link: false,
            },
            &mut s,
        );
        assert!(matches!(result, ExecuteVerdict::Continue));
        assert_eq!(s.pc, 0x1000); // unchanged
    }

    #[test]
    fn divdu_basic() {
        let mut s = PpuState::new();
        s.gpr[3] = 100;
        s.gpr[4] = 7;
        exec_no_mem(
            &PpuInstruction::Divdu {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 14);
    }

    #[test]
    fn divdu_divide_by_zero() {
        let mut s = PpuState::new();
        s.gpr[3] = 100;
        s.gpr[4] = 0;
        exec_no_mem(
            &PpuInstruction::Divdu {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 0);
    }

    #[test]
    fn divdu_large_values() {
        let mut s = PpuState::new();
        s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
        s.gpr[4] = 2;
        exec_no_mem(
            &PpuInstruction::Divdu {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 0x7FFF_FFFF_FFFF_FFFF);
    }

    #[test]
    fn divd_signed() {
        let mut s = PpuState::new();
        s.gpr[3] = (-100i64) as u64;
        s.gpr[4] = 7;
        exec_no_mem(
            &PpuInstruction::Divd {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5] as i64, -14);
    }

    #[test]
    fn divd_divide_by_zero() {
        let mut s = PpuState::new();
        s.gpr[3] = 100;
        s.gpr[4] = 0;
        exec_no_mem(
            &PpuInstruction::Divd {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 0);
    }

    #[test]
    fn mulld_basic() {
        let mut s = PpuState::new();
        s.gpr[3] = 7;
        s.gpr[4] = 8;
        exec_no_mem(
            &PpuInstruction::Mulld {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 56);
    }

    #[test]
    fn mulld_wraps_on_overflow() {
        let mut s = PpuState::new();
        s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
        s.gpr[4] = 2;
        exec_no_mem(
            &PpuInstruction::Mulld {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
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
        exec_no_mem(
            &PpuInstruction::Adde {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
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
        exec_no_mem(
            &PpuInstruction::Adde {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
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
        exec_no_mem(
            &PpuInstruction::Mulhdu {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 1);
    }

    #[test]
    fn mulhdu_small_product_is_zero() {
        // 7 * 8 = 56; fits in 64 bits, so high 64 bits = 0.
        let mut s = PpuState::new();
        s.gpr[3] = 7;
        s.gpr[4] = 8;
        exec_no_mem(
            &PpuInstruction::Mulhdu {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 0);
    }

    #[test]
    fn ldarx_loads_from_memory() {
        let mut mem = vec![0u8; 0x2000];
        mem[0x1008..0x1010].copy_from_slice(&0xDEAD_BEEF_CAFE_BABEu64.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x8;
        let mut effects = Vec::new();
        let result = exec_with_mem(
            &PpuInstruction::Ldarx {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(result, ExecuteVerdict::Continue);
        assert_eq!(s.gpr[5], 0xDEAD_BEEF_CAFE_BABE);
    }

    #[test]
    fn stdcx_always_succeeds_in_single_threaded() {
        let mut s = PpuState::new();
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x8;
        s.gpr[5] = 0xDEAD_BEEF_CAFE_BABE;
        let mut effects = Vec::new();
        let result = exec_with_mem(
            &PpuInstruction::Stdcx {
                rs: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            0,
            &[0u8; 0x2000],
            &mut effects,
        );
        assert_eq!(result, ExecuteVerdict::Continue);
        // CR0 EQ must be set to indicate success.
        assert_eq!(s.cr_field(0), 0b0010);
        // Should have emitted one store effect.
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x1008);
                assert_eq!(bytes.bytes(), &0xDEAD_BEEF_CAFE_BABEu64.to_be_bytes());
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    // stfiwx / stfsu / stfdu / mulhw / cntlzd / addze / orc: one
    // exec test per variant pinning the semantics.

    #[test]
    fn mulhw_signed_high_32_bits() {
        // -2 * 3 = -6; high 32 bits sign-extended == 0xFFFFFFFF.
        let mut s = PpuState::new();
        s.gpr[4] = (-2i32) as u32 as u64;
        s.gpr[5] = 3;
        exec_no_mem(
            &PpuInstruction::Mulhw {
                rt: 3,
                ra: 4,
                rb: 5,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 0xFFFFFFFF_FFFFFFFFu64);
    }

    #[test]
    fn mulhw_positive_produces_zero_high_bits() {
        let mut s = PpuState::new();
        s.gpr[4] = 0x0001_0000;
        s.gpr[5] = 0x0001_0000;
        exec_no_mem(
            &PpuInstruction::Mulhw {
                rt: 3,
                ra: 4,
                rb: 5,
            },
            &mut s,
        );
        // 0x10000 * 0x10000 = 0x1_0000_0000; high 32 = 1.
        assert_eq!(s.gpr[3], 1);
    }

    #[test]
    fn cntlzd_counts_64_for_zero() {
        let mut s = PpuState::new();
        s.gpr[5] = 0;
        exec_no_mem(&PpuInstruction::Cntlzd { ra: 3, rs: 5 }, &mut s);
        assert_eq!(s.gpr[3], 64);
    }

    #[test]
    fn cntlzd_high_bit_set_returns_zero() {
        let mut s = PpuState::new();
        s.gpr[5] = 1u64 << 63;
        exec_no_mem(&PpuInstruction::Cntlzd { ra: 3, rs: 5 }, &mut s);
        assert_eq!(s.gpr[3], 0);
    }

    #[test]
    fn addze_with_ca_zero_copies_ra() {
        let mut s = PpuState::new();
        s.gpr[4] = 42;
        s.set_xer_ca(false);
        exec_no_mem(&PpuInstruction::Addze { rt: 3, ra: 4 }, &mut s);
        assert_eq!(s.gpr[3], 42);
        assert!(!s.xer_ca());
    }

    #[test]
    fn addze_with_ca_set_adds_one() {
        let mut s = PpuState::new();
        s.gpr[4] = 42;
        s.set_xer_ca(true);
        exec_no_mem(&PpuInstruction::Addze { rt: 3, ra: 4 }, &mut s);
        assert_eq!(s.gpr[3], 43);
        assert!(!s.xer_ca());
    }

    #[test]
    fn addze_overflow_sets_ca() {
        let mut s = PpuState::new();
        s.gpr[4] = u64::MAX;
        s.set_xer_ca(true);
        exec_no_mem(&PpuInstruction::Addze { rt: 3, ra: 4 }, &mut s);
        assert_eq!(s.gpr[3], 0);
        assert!(s.xer_ca());
    }

    #[test]
    fn orc_is_or_with_complement_rb() {
        let mut s = PpuState::new();
        s.gpr[4] = 0x00FF_0000;
        s.gpr[5] = 0x0000_00FF;
        exec_no_mem(
            &PpuInstruction::Orc {
                ra: 3,
                rs: 4,
                rb: 5,
            },
            &mut s,
        );
        // 0x00FF_0000 | !0x0000_00FF == 0xFFFF_FF00 sign-extended to u64
        assert_eq!(s.gpr[3], 0xFFFF_FFFF_FFFF_FF00);
    }

    #[test]
    fn stfsu_updates_ra_and_emits_store_effect() {
        let mut s = PpuState::new();
        s.gpr[8] = 0x2000;
        s.fpr[13] = 0x4000_0000_0000_0000;
        let mut effects = Vec::new();
        let out = exec_with_mem(
            &PpuInstruction::Stfsu {
                frs: 13,
                ra: 8,
                imm: 8,
            },
            &mut s,
            0,
            &[0u8; 0x4000],
            &mut effects,
        );
        assert_eq!(out, ExecuteVerdict::Continue);
        assert_eq!(s.gpr[8], 0x2008, "ra is updated to ea");
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::SharedWriteIntent { range, .. } => {
                assert_eq!(range.start().raw(), 0x2008);
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn stfdu_updates_ra_and_emits_store_effect() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.fpr[1] = 0xDEAD_BEEF_CAFE_BABE;
        let mut effects = Vec::new();
        let out = exec_with_mem(
            &PpuInstruction::Stfdu {
                frs: 1,
                ra: 1,
                imm: -8,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        assert_eq!(out, ExecuteVerdict::Continue);
        assert_eq!(s.gpr[1], 0xF8);
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0xF8);
                assert_eq!(bytes.bytes(), &0xDEAD_BEEF_CAFE_BABEu64.to_be_bytes());
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn stfiwx_stores_low_32_bits_of_fpr_as_integer_word() {
        // Unlike Stfs (single-precision round-convert), stfiwx writes
        // the low 32 bits of the FPR bit pattern verbatim as a u32.
        let mut s = PpuState::new();
        s.gpr[4] = 0x1000;
        s.gpr[5] = 0x20;
        s.fpr[13] = 0x4040_4040_1234_5678;
        let mut effects = Vec::new();
        let out = exec_with_mem(
            &PpuInstruction::Stfiwx {
                frs: 13,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &[0u8; 0x2000],
            &mut effects,
        );
        assert_eq!(out, ExecuteVerdict::Continue);
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x1020);
                assert_eq!(
                    bytes.bytes(),
                    &0x1234_5678u32.to_be_bytes(),
                    "low 32 bits verbatim"
                );
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    // -- Quickened instruction tests --

    #[test]
    fn li_matches_addi_ra0() {
        let mut s1 = PpuState::new();
        exec_no_mem(
            &PpuInstruction::Addi {
                rt: 3,
                ra: 0,
                imm: 42,
            },
            &mut s1,
        );

        let mut s2 = PpuState::new();
        exec_no_mem(&PpuInstruction::Li { rt: 3, imm: 42 }, &mut s2);

        assert_eq!(s1.gpr[3], s2.gpr[3]);
        assert_eq!(s2.gpr[3], 42);
    }

    #[test]
    fn li_negative_sign_extends() {
        let mut s = PpuState::new();
        exec_no_mem(&PpuInstruction::Li { rt: 5, imm: -1 }, &mut s);
        assert_eq!(s.gpr[5], u64::MAX);
    }

    #[test]
    fn mr_matches_or_same_reg() {
        let mut s1 = PpuState::new();
        s1.gpr[4] = 0xDEAD_BEEF;
        exec_no_mem(
            &PpuInstruction::Or {
                ra: 3,
                rs: 4,
                rb: 4,
            },
            &mut s1,
        );

        let mut s2 = PpuState::new();
        s2.gpr[4] = 0xDEAD_BEEF;
        exec_no_mem(&PpuInstruction::Mr { ra: 3, rs: 4 }, &mut s2);

        assert_eq!(s1.gpr[3], s2.gpr[3]);
        assert_eq!(s2.gpr[3], 0xDEAD_BEEF);
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
        // clrlwi clears the top 16 bits of the 32-bit value
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

    // =================================================================
    // Superinstruction tests
    // =================================================================

    #[test]
    fn lwz_cmpwi_matches_separate_execution() {
        // Execute lwz + cmpwi separately
        let mut mem = vec![0u8; 0x2000];
        mem[0x1008..0x100C].copy_from_slice(&42u32.to_be_bytes());
        let mut s1 = PpuState::new();
        s1.gpr[1] = 0x1000;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lwz {
                rt: 3,
                ra: 1,
                imm: 8,
            },
            &mut s1,
            0,
            &mem,
            &mut effects,
        );
        exec_no_mem(
            &PpuInstruction::Cmpwi {
                bf: 0,
                ra: 3,
                imm: 42,
            },
            &mut s1,
        );

        // Execute fused LwzCmpwi
        let mut s2 = PpuState::new();
        s2.gpr[1] = 0x1000;
        let mut effects2 = Vec::new();
        exec_with_mem(
            &PpuInstruction::LwzCmpwi {
                rt: 3,
                ra_load: 1,
                offset: 8,
                bf: 0,
                cmp_imm: 42,
            },
            &mut s2,
            0,
            &mem,
            &mut effects2,
        );

        assert_eq!(s1.gpr[3], s2.gpr[3]);
        assert_eq!(s1.cr, s2.cr);
        assert_eq!(s2.gpr[3], 42);
        assert_eq!(s2.cr_field(0), 0b0010); // EQ
    }

    #[test]
    fn lwz_cmpwi_lt_and_gt() {
        let mut mem = vec![0u8; 0x2000];
        // Store value 5 (less than 10)
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

        // Store value 20 (greater than 10)
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
    fn li_stw_matches_separate_execution() {
        // Execute li + stw separately
        let mut s1 = PpuState::new();
        s1.gpr[1] = 0x1000;
        let mut effects1 = Vec::new();
        exec_with_mem(
            &PpuInstruction::Li { rt: 5, imm: 99 },
            &mut s1,
            0,
            &[0u8; 0x2000],
            &mut effects1,
        );
        exec_with_mem(
            &PpuInstruction::Stw {
                rs: 5,
                ra: 1,
                imm: 0,
            },
            &mut s1,
            0,
            &[0u8; 0x2000],
            &mut effects1,
        );

        // Execute fused LiStw
        let mut s2 = PpuState::new();
        s2.gpr[1] = 0x1000;
        let mut effects2 = Vec::new();
        exec_with_mem(
            &PpuInstruction::LiStw {
                rt: 5,
                imm: 99,
                ra_store: 1,
                store_offset: 0,
            },
            &mut s2,
            0,
            &[0u8; 0x2000],
            &mut effects2,
        );

        assert_eq!(s1.gpr[5], s2.gpr[5]);
        assert_eq!(s2.gpr[5], 99);
        // Both should produce a store effect at address 0x1000
        assert!(!effects1.is_empty());
        assert!(!effects2.is_empty());
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
    fn mflr_stw_matches_separate_execution() {
        // Execute mflr + stw separately
        let mut s1 = PpuState::new();
        s1.lr = 0x0040_0100;
        s1.gpr[1] = 0x1000;
        let mut effects1 = Vec::new();
        exec_with_mem(
            &PpuInstruction::Mflr { rt: 0 },
            &mut s1,
            0,
            &[0u8; 0x2000],
            &mut effects1,
        );
        exec_with_mem(
            &PpuInstruction::Stw {
                rs: 0,
                ra: 1,
                imm: 16,
            },
            &mut s1,
            0,
            &[0u8; 0x2000],
            &mut effects1,
        );

        // Execute fused MflrStw
        let mut s2 = PpuState::new();
        s2.lr = 0x0040_0100;
        s2.gpr[1] = 0x1000;
        let mut effects2 = Vec::new();
        exec_with_mem(
            &PpuInstruction::MflrStw {
                rt: 0,
                ra_store: 1,
                store_offset: 16,
            },
            &mut s2,
            0,
            &[0u8; 0x2000],
            &mut effects2,
        );

        assert_eq!(s1.gpr[0], s2.gpr[0]);
        assert_eq!(s2.gpr[0], 0x0040_0100);
        assert!(!effects1.is_empty());
        assert!(!effects2.is_empty());
    }

    #[test]
    fn lwz_mtlr_matches_separate_execution() {
        // Execute lwz + mtlr separately
        let mut mem = vec![0u8; 0x2000];
        mem[0x1010..0x1014].copy_from_slice(&0x0040_0100u32.to_be_bytes());
        let mut s1 = PpuState::new();
        s1.gpr[1] = 0x1000;
        let mut effects1 = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lwz {
                rt: 0,
                ra: 1,
                imm: 16,
            },
            &mut s1,
            0,
            &mem,
            &mut effects1,
        );
        exec_no_mem(&PpuInstruction::Mtlr { rs: 0 }, &mut s1);

        // Execute fused LwzMtlr
        let mut s2 = PpuState::new();
        s2.gpr[1] = 0x1000;
        let mut effects2 = Vec::new();
        exec_with_mem(
            &PpuInstruction::LwzMtlr {
                rt: 0,
                ra_load: 1,
                offset: 16,
            },
            &mut s2,
            0,
            &mem,
            &mut effects2,
        );

        assert_eq!(s1.gpr[0], s2.gpr[0]);
        assert_eq!(s1.lr, s2.lr);
        assert_eq!(s2.gpr[0], 0x0040_0100);
        assert_eq!(s2.lr, 0x0040_0100);
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
}
