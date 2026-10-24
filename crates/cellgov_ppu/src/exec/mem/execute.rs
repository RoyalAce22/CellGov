//! The memory-instruction dispatch: one match over every load, store, atomic and cache-block form.

use crate::exec::memory_helpers::{
    buffer_conditional_store, buffer_store, load_ze, unmapped, LoadPort, Width,
};
use crate::exec::{ExecuteVerdict, PpuFault};
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_sync::ReservedLine;

use cellgov_ps3_abi::hw::ppu::DCBZ_BLOCK_BYTES;

use super::helpers::{holds_reservation_for, read_aligned_16};
use super::scalar::{load, scalar, store, Scalar};
use super::string::{string_load, string_store};

pub(crate) fn execute(
    insn: &PpuInstruction,
    state: &mut PpuState,
    unit_id: UnitId,
    region_views: &[cellgov_mem::RegionView<'_>],
    effects: &mut Vec<Effect>,
    store_buf: &mut StoreBuffer,
) -> ExecuteVerdict {
    match scalar(insn) {
        Some(Scalar::Load(ea, width, dest, update)) => {
            let mut port = LoadPort::new(region_views, store_buf, effects, unit_id);
            return load(state, &mut port, ea, width, dest, update);
        }
        Some(Scalar::Store(ea, width, src, update)) => {
            return store(state, store_buf, ea, width, src, update);
        }
        None => {}
    }
    match *insn {
        // [PPC-Book1 p:46 s:3.3.5] lmw D-form: for r=RT..31, GPR[r] =
        // zero-extend(MEM(EA,4)); EA += 4. Invalid form if RA is in
        // (RT..=31) or RA==0.
        PpuInstruction::Lmw { rt, ra, imm } => {
            // [CBE-Handbook p:254 s:9.5.9] The PPE loads up to the colliding register, then takes the illegal-instruction interrupt; the fault discards the partial loads, so no load runs first.
            if ra == 0 || ra >= rt {
                return ExecuteVerdict::Fault(PpuFault::InvalidForm("lmw"));
            }
            let mut ea = state.ea_d_form(ra, imm);
            let mut port = LoadPort::new(region_views, store_buf, effects, unit_id);
            for r in (rt as usize)..32 {
                match load_ze(&mut port, ea, Width::B4) {
                    Ok(val) => {
                        state.set_gpr(r, val);
                        ea = ea.wrapping_add(4);
                    }
                    Err(ea) => return ExecuteVerdict::MemFault(ea),
                }
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:54 s:3.3] stmw D-form: for r=RS..31, MEM(EA,4) = low32(GPR[r]); EA += 4.
        // Capacity is pre-checked so a mid-instruction BufferFull
        // cannot leave a partially-committed multi-store; retry
        // would duplicate the earlier word writes.
        PpuInstruction::Stmw { rs, ra, imm } => {
            let first = rs as usize;
            let ea = state.ea_d_form(ra, imm);
            let gpr = *state.gpr.as_array();
            let words = (first..32).map(|r| (ea.wrapping_add(4 * (r - first) as u64), 4, gpr[r]));
            store_prechecked(store_buf, state, 32 - first, words, "stmw word")
        }

        // [PPC-Book1 p:55 s:3.3.5] String moves transfer N bytes
        // packed four-per-register, MSB-first into the low 32 bits
        // of each successive GPR (high 32 bits zeroed); the register
        // sequence wraps at r31 -> r0. lswi / stswi use NB from the
        // encoding (0 means 32); lswx / stswx use the byte count
        // from XER[57:63] (0 is a no-op).
        PpuInstruction::Lswi { rt, ra, nb } => {
            let n = if nb == 0 { 32usize } else { nb as usize };
            let base = state.ra_or_zero(ra);
            let mut port = LoadPort::new(region_views, store_buf, effects, unit_id);
            string_load(state, &mut port, rt as usize, base, n)
        }
        PpuInstruction::Lswx { rt, ra, rb } => {
            let base = state.ea_x_form(ra, rb);
            let n = state.xer_tbc() as usize;
            let mut port = LoadPort::new(region_views, store_buf, effects, unit_id);
            string_load(state, &mut port, rt as usize, base, n)
        }
        PpuInstruction::Stswi { rs, ra, nb } => {
            let n = if nb == 0 { 32usize } else { nb as usize };
            let base = state.ra_or_zero(ra);
            string_store(state, store_buf, rs as usize, base, n)
        }
        PpuInstruction::Stswx { rs, ra, rb } => {
            let base = state.ea_x_form(ra, rb);
            let n = state.xer_tbc() as usize;
            string_store(state, store_buf, rs as usize, base, n)
        }

        // Atomic load-reserve / store-conditional
        // [PPC-Book2 p:24 s:3.3] lwarx/ldarx: load + set RESERVE, RESERVE_ADDR = real_addr(EA); EA must be naturally aligned.
        PpuInstruction::Ldarx { rt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            state.ldarx_executed = state.ldarx_executed.wrapping_add(1);
            load_reserve(
                state,
                region_views,
                store_buf,
                effects,
                unit_id,
                rt,
                ea,
                Width::B8,
            )
        }
        PpuInstruction::Lwarx { rt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            state.lwarx_executed = state.lwarx_executed.wrapping_add(1);
            load_reserve(
                state,
                region_views,
                store_buf,
                effects,
                unit_id,
                rt,
                ea,
                Width::B4,
            )
        }
        // [PPC-Book2 p:25 s:3.3] stwcx./stdcx.: if RESERVE && RESERVE_ADDR == real_addr(EA) then store + CR0 = 0b00||1||XER[SO], else CR0 = 0b00||0||XER[SO]; reservation cleared.
        PpuInstruction::Stdcx { rs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            state.stdcx_executed = state.stdcx_executed.wrapping_add(1);
            store_conditional(state, store_buf, effects, rs, ea, Width::B8)
        }
        PpuInstruction::Stwcx { rs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            state.stwcx_executed = state.stwcx_executed.wrapping_add(1);
            store_conditional(state, store_buf, effects, rs, ea, Width::B4)
        }

        // Vector loads / stores
        // [CBE-Handbook p:744 s:A.3] PPE-only VMX additions lvlx/lvrx/stvlx/stvrx: unaligned vector load/store helpers.
        // [CBE-Handbook p:744 s:A.3.3] lvlxl / lvrxl: identical to lvlx / lvrx,
        // with the LRU cache hint that CellGov's no-cache model ignores.
        PpuInstruction::Lvlx { vt, ra, rb } | PpuInstruction::Lvlxl { vt, ra, rb } => {
            let addr = state.ea_x_form(ra, rb);
            let mut port = LoadPort::new(region_views, store_buf, effects, unit_id);
            let val = match read_aligned_16(&mut port, addr & !15u64) {
                Ok(v) => v,
                Err(ea) => return ExecuteVerdict::MemFault(ea),
            };
            let shift = ((addr & 15) * 8) as u32;
            state.set_vr(vt as usize, if shift == 0 { val } else { val << shift });
            ExecuteVerdict::Continue
        }
        PpuInstruction::Lvrx { vt, ra, rb } | PpuInstruction::Lvrxl { vt, ra, rb } => {
            let addr = state.ea_x_form(ra, rb);
            let lo = addr & 15;
            // [CBE-Handbook p:744 s:A.3.3 Table A-9] a quadword-aligned
            // lvrx makes no attempt to access storage and delivers
            // zero, so it neither faults on an unmapped line nor
            // records a read of one.
            if lo == 0 {
                state.set_vr(vt as usize, 0);
                return ExecuteVerdict::Continue;
            }
            let mut port = LoadPort::new(region_views, store_buf, effects, unit_id);
            let val = match read_aligned_16(&mut port, addr & !15u64) {
                Ok(v) => v,
                Err(ea) => return ExecuteVerdict::MemFault(ea),
            };
            state.set_vr(vt as usize, val >> ((16 - lo) * 8) as u32);
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:6-21 s:6.2] Load Vector Indexed (lvx, X-form): EA = ((RA|0)+(RB)) & ~0xF; MEM(EA,16) -> vT.
        // [AltiVec-PEM p:6-23 s:6.2] lvxl: same semantics as lvx; the "Last" suffix is a cache LRU
        // hint that CellGov's no-cache model ignores.
        PpuInstruction::Lvx { vt, ra, rb } | PpuInstruction::Lvxl { vt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb) & !15u64;
            let mut port = LoadPort::new(region_views, store_buf, effects, unit_id);
            let val = match read_aligned_16(&mut port, ea) {
                Ok(v) => v,
                Err(ea) => return ExecuteVerdict::MemFault(ea),
            };
            state.set_vr(vt as usize, val);
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:6-21 s:6.2] lvsl: VRT[i] = sh + i for i in 0..16, where sh = EA[60:63].
        // Memory is not read; the result is a permute control vector derived from the low 4 bits of EA.
        PpuInstruction::Lvsl { vt, ra, rb } => {
            let sh = (state.ea_x_form(ra, rb) & 0xF) as u8;
            let mut bytes = [0u8; 16];
            for (i, b) in bytes.iter_mut().enumerate() {
                *b = sh.wrapping_add(i as u8);
            }
            state.set_vr(vt as usize, u128::from_be_bytes(bytes));
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:6-22 s:6.2] lvsr: VRT[i] = 16 + i - sh for i in 0..16, where sh = EA[60:63].
        // Memory is not read; symmetric companion to lvsl.
        PpuInstruction::Lvsr { vt, ra, rb } => {
            let sh = (state.ea_x_form(ra, rb) & 0xF) as u8;
            let mut bytes = [0u8; 16];
            for (i, b) in bytes.iter_mut().enumerate() {
                *b = 16u8.wrapping_add(i as u8).wrapping_sub(sh);
            }
            state.set_vr(vt as usize, u128::from_be_bytes(bytes));
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:6-15 s:6.2] lvebx: byte load at EA into byte position (EA & 0xF) of VRT.
        // Other byte lanes are spec-undefined; we preserve them.
        PpuInstruction::Lvebx { vt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            let m = (ea & 0xF) as usize;
            let byte = match load_ze(
                &mut LoadPort::new(region_views, store_buf, effects, unit_id),
                ea,
                Width::B1,
            ) {
                Ok(v) => v as u8,
                Err(e) => return ExecuteVerdict::MemFault(e),
            };
            let mut bytes = state.vr[vt as usize].to_be_bytes();
            bytes[m] = byte;
            state.set_vr(vt as usize, u128::from_be_bytes(bytes));
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:6-16 s:6.2] lvehx: halfword load at (EA & ~1) into halfword position
        // ((EA & 0xE) / 2) of VRT; other lanes preserved (spec-undefined).
        PpuInstruction::Lvehx { vt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb) & !1u64;
            let m = (ea & 0xF) as usize;
            let val = match load_ze(
                &mut LoadPort::new(region_views, store_buf, effects, unit_id),
                ea,
                Width::B2,
            ) {
                Ok(v) => v as u16,
                Err(e) => return ExecuteVerdict::MemFault(e),
            };
            let mut bytes = state.vr[vt as usize].to_be_bytes();
            let hb = val.to_be_bytes();
            bytes[m] = hb[0];
            bytes[m + 1] = hb[1];
            state.set_vr(vt as usize, u128::from_be_bytes(bytes));
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:6-17 s:6.2] lvewx: word load at (EA & ~3) into word position
        // ((EA & 0xC) / 4) of VRT; other lanes preserved (spec-undefined).
        PpuInstruction::Lvewx { vt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb) & !3u64;
            let m = (ea & 0xF) as usize;
            let val = match load_ze(
                &mut LoadPort::new(region_views, store_buf, effects, unit_id),
                ea,
                Width::B4,
            ) {
                Ok(v) => v as u32,
                Err(e) => return ExecuteVerdict::MemFault(e),
            };
            let mut bytes = state.vr[vt as usize].to_be_bytes();
            let wb = val.to_be_bytes();
            bytes[m..m + 4].copy_from_slice(&wb);
            state.set_vr(vt as usize, u128::from_be_bytes(bytes));
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:6-29 s:6.2] stvebx: byte at byte-position (EA & 0xF) of VS -> MEM(EA, 1).
        PpuInstruction::Stvebx { vs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            let m = (ea & 0xF) as usize;
            let byte = state.vr[vs as usize].to_be_bytes()[m];
            buffer_store(store_buf, state, ea, 1, byte as u64)
        }
        // [AltiVec-PEM p:6-30 s:6.2] stvehx: halfword at lane (EA & 0xE) of VS -> MEM(EA & ~1, 2).
        PpuInstruction::Stvehx { vs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb) & !1u64;
            let m = (ea & 0xF) as usize;
            let bytes = state.vr[vs as usize].to_be_bytes();
            let val = u16::from_be_bytes([bytes[m], bytes[m + 1]]);
            buffer_store(store_buf, state, ea, 2, val as u64)
        }
        // [AltiVec-PEM p:6-31 s:6.2] stvewx: word at lane (EA & 0xC) of VS -> MEM(EA & ~3, 4).
        PpuInstruction::Stvewx { vs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb) & !3u64;
            let m = (ea & 0xF) as usize;
            let bytes = state.vr[vs as usize].to_be_bytes();
            let val = u32::from_be_bytes([bytes[m], bytes[m + 1], bytes[m + 2], bytes[m + 3]]);
            buffer_store(store_buf, state, ea, 4, val as u64)
        }
        // [CBE-Handbook p:744 s:A.3.3] stvlx / stvrx: partial-vector stores. stvlx writes
        // the high `16 - (EA & 0xF)` bytes at EA; stvrx writes the low `EA & 0xF` bytes
        // at the aligned line below EA. Capacity pre-check prevents partial commit on
        // mid-instruction BufferFull (retry would duplicate prior bytes' SharedWriteIntent).
        // [CBE-Handbook p:744 s:A.3.3] stvlxl / stvrxl: identical to stvlx / stvrx; the
        // "Last" suffix is an LRU cache hint CellGov's no-cache model ignores.
        PpuInstruction::Stvlx { vs, ra, rb } | PpuInstruction::Stvlxl { vs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            let count = 16 - (ea & 0xF) as usize;
            let bytes = state.vr[vs as usize].to_be_bytes();
            let stores = (0..count).map(|i| (ea + i as u64, 1, u64::from(bytes[i])));
            store_prechecked(store_buf, state, count, stores, "stvlx byte")
        }
        PpuInstruction::Stvrx { vs, ra, rb } | PpuInstruction::Stvrxl { vs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            let m = (ea & 0xF) as usize;
            if m == 0 {
                return ExecuteVerdict::Continue;
            }
            let aligned = ea & !15u64;
            let bytes = state.vr[vs as usize].to_be_bytes();
            let stores = (0..m).map(|i| (aligned + i as u64, 1, u64::from(bytes[16 - m + i])));
            store_prechecked(store_buf, state, m, stores, "stvrx byte")
        }
        // [AltiVec-PEM p:6-28 s:6.2] Store Vector Indexed (stvx, X-form): EA = ((RA|0)+(RB)) & ~0xF; vS -> MEM(EA,16).
        // [AltiVec-PEM p:6-33 s:6.2] stvxl: identical to stvx; the LRU "Last" suffix is a cache
        // hint CellGov's no-cache model ignores.
        PpuInstruction::Stvx { vs, ra, rb } | PpuInstruction::Stvxl { vs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb) & !15u64;
            // Split into two 8-byte halves so the reservation
            // clear-sweep in buffer_store covers both. Capacity
            // pre-check prevents partial commit on `BufferFull`.
            let v = state.vr[vs as usize];
            let halves = [(ea, 8, (v >> 64) as u64), (ea + 8, 8, v as u64)];
            store_prechecked(store_buf, state, 2, halves.into_iter(), "stvx half")
        }

        // Cache control
        // [PPC-Book2 p:20 s:3.2] Data Cache Block set to Zero (dcbz, X-form): zero the block of size n containing EA; treated as a Store.
        PpuInstruction::Dcbz { ra, rb } => {
            let ea = state.ea_x_form(ra, rb) & !(DCBZ_BLOCK_BYTES as u64 - 1);
            state.dcbz_executed = state.dcbz_executed.wrapping_add(1);
            debug_assert!(
                !(0xC000_0000..0xC010_0000).contains(&ea),
                "dcbz into RSX MMIO window at 0x{ea:x} likely indicates pointer corruption",
            );
            // Capacity pre-check prevents a partial block from
            // committing on `BufferFull` mid-loop.
            const DCBZ_STORES: usize = DCBZ_BLOCK_BYTES / 8;
            let zeros = (0..DCBZ_STORES).map(|i| (ea + (i as u64) * 8, 8, 0));
            store_prechecked(store_buf, state, DCBZ_STORES, zeros, "dcbz")
        }

        _ => unreachable!("mem::execute called with non-memory variant"),
    }
}

/// lwarx / ldarx after the counter bump: alignment fault, load, then the
/// reservation and its `ReservationAcquire` effect.
#[inline]
#[allow(clippy::too_many_arguments)]
fn load_reserve(
    state: &mut PpuState,
    region_views: &[cellgov_mem::RegionView<'_>],
    store_buf: &StoreBuffer,
    effects: &mut Vec<Effect>,
    unit_id: UnitId,
    rt: u8,
    ea: u64,
    width: Width,
) -> ExecuteVerdict {
    if ea & u64::from(width.bytes() - 1) != 0 {
        return ExecuteVerdict::Fault(PpuFault::AlignmentInterrupt(ea));
    }
    let loaded = load_ze(
        &mut LoadPort::new(region_views, store_buf, effects, unit_id),
        ea,
        width,
    );
    match loaded {
        Ok(val) => {
            state.set_gpr(rt as usize, val);
            let line = ReservedLine::containing(ea);
            state.set_reservation(Some(line));
            effects.push(Effect::ReservationAcquire {
                line_addr: line.addr(),
                source: unit_id,
            });
            ExecuteVerdict::Continue
        }
        Err(ea) => ExecuteVerdict::MemFault(ea),
    }
}

/// stwcx. / stdcx. after the counter bump.
///
/// The local reservation is authoritative: cross-unit clears happen at
/// step start, and the unit's own stores leave it.
#[inline]
fn store_conditional(
    state: &mut PpuState,
    store_buf: &mut StoreBuffer,
    effects: &[Effect],
    rs: u8,
    ea: u64,
    width: Width,
) -> ExecuteVerdict {
    let size = width.bytes();
    if ea & u64::from(size - 1) != 0 {
        return ExecuteVerdict::Fault(PpuFault::AlignmentInterrupt(ea));
    }
    let success = holds_reservation_for(state, ea);
    // [PPC-Book2 p:25 s:3.3.2 Atomic Update Primitives] CR0 = 0b00 || n || XER[SO].
    let so = u8::from(state.xer_so());
    if success {
        if ByteRange::new(GuestAddr::new(ea), u64::from(size)).is_none() {
            return ExecuteVerdict::MemFault(unmapped(ea));
        }
        // Buffered so the `ConditionalStore` lands in program
        // order (see `StoreBuffer::insert_conditional`); the
        // capacity check comes before CR0 so a full buffer
        // retries with CR0 and the reservation untouched.
        if !store_buf.has_capacity_for(1) {
            return ExecuteVerdict::BufferFull;
        }
        state.set_cr_field(0, 0b0010 | so);
        let value = match width {
            Width::B8 => state.gpr[rs as usize],
            _ => u64::from(state.gpr[rs as usize] as u32),
        };
        let staged = buffer_conditional_store(store_buf, ea, size, value, effects.len());
        debug_assert!(
            staged != ExecuteVerdict::BufferFull,
            "store-conditional insert after has_capacity_for(1) passed"
        );
        if staged != ExecuteVerdict::Continue {
            return staged;
        }
    } else {
        state.set_cr_field(0, so);
    }
    state.set_reservation(None);
    ExecuteVerdict::Continue
}

/// Stage `count` stores of `(ea, size, value)` behind one capacity
/// check, so a `BufferFull` never leaves part of the instruction
/// staged; a retry would duplicate the earlier writes.
#[inline]
fn store_prechecked(
    store_buf: &mut StoreBuffer,
    state: &mut PpuState,
    count: usize,
    stores: impl Iterator<Item = (u64, u8, u64)>,
    what: &str,
) -> ExecuteVerdict {
    if !store_buf.has_capacity_for(count) {
        return ExecuteVerdict::BufferFull;
    }
    for (ea, size, value) in stores {
        let v = buffer_store(store_buf, state, ea, size, value);
        debug_assert!(
            v != ExecuteVerdict::BufferFull,
            "{what} store failed after capacity pre-check"
        );
        if v != ExecuteVerdict::Continue {
            return v;
        }
    }
    ExecuteVerdict::Continue
}
