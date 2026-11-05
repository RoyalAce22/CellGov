//! The memory-instruction dispatch: one match over every load, store, atomic and cache-block form.

use crate::exec::memory_helpers::{
    buffer_conditional_store, buffer_store, caching_inhibited, check_block_translates, check_store,
    inhibited_store, load_ze, LoadPort, Width,
};
use crate::exec::{ExecuteVerdict, PpuFault};
use crate::instruction::PpuInstruction;
use crate::state::{effective_address, PpuState};
use crate::store_buffer::StoreBuffer;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_sync::ReservedLine;

use cellgov_ps3_abi::hw::ppu::{DCBZ_BLOCK_BYTES, PPU_USER_MSR};

// Every load and store reads and writes memory big-endian.
const _: () = assert!(
    !PPU_USER_MSR.le,
    "the memory arms implement big-endian mode only"
);

use super::helpers::{holds_reservation_for, read_aligned_16};
use super::scalar::{load, scalar, store, Scalar};
use super::string::{string_load, string_loads_into, string_store};

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
            return store(state, region_views, store_buf, ea, width, src, update);
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
                return ExecuteVerdict::Fault(PpuFault::illegal("lmw"));
            }
            let ea = state.ea_d_form(ra, imm);
            // [CBE-Handbook p:253 s:9.5.8] An lmw or stmw operand that is not word-aligned takes the alignment interrupt.
            if ea & 3 != 0 {
                return ExecuteVerdict::Fault(PpuFault::alignment(ea, false));
            }
            // [CBE-Handbook p:253 s:9.5.8] An lmw, stmw, lswi, lswx, stswi or stswx operand in caching-inhibited storage takes the alignment interrupt.
            if caching_inhibited(region_views, ea, 4 * (32 - u64::from(rt))) {
                return ExecuteVerdict::Fault(PpuFault::alignment(ea, false));
            }
            // The loop loads every word before it writes a register, so
            // a fault part way writes none.
            let mut port = LoadPort::new(region_views, store_buf, effects, unit_id);
            let mut words = [0u64; 32];
            for (i, word) in words.iter_mut().enumerate().skip(rt as usize) {
                let at = ea.wrapping_add(4 * (i - rt as usize) as u64);
                match load_ze(&mut port, at, Width::B4) {
                    Ok(val) => *word = val,
                    Err(ea) => return ExecuteVerdict::MemFault(ea),
                }
            }
            for (r, &word) in words.iter().enumerate().skip(rt as usize) {
                state.set_gpr(r, word);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:54 s:3.3] stmw D-form: for r=RS..31, MEM(EA,4) = low32(GPR[r]); EA += 4.
        // A misaligned EA takes the alignment interrupt, as for lmw.
        // Capacity is pre-checked so a mid-instruction BufferFull
        // cannot leave a partially-committed multi-store; retry
        // would duplicate the earlier word writes.
        PpuInstruction::Stmw { rs, ra, imm } => {
            let first = rs as usize;
            let ea = state.ea_d_form(ra, imm);
            if ea & 3 != 0 {
                return ExecuteVerdict::Fault(PpuFault::alignment(ea, true));
            }
            let len = 4 * (32 - first) as u64;
            if caching_inhibited(region_views, ea, len) {
                return inhibited_store(region_views, ea, ea, len);
            }
            let gpr = *state.gpr.as_array();
            let words = (first..32).map(|r| (ea.wrapping_add(4 * (r - first) as u64), 4, gpr[r]));
            store_prechecked(store_buf, region_views, 32 - first, words, "stmw word")
        }

        // [PPC-Book1 p:55 s:3.3.5] String moves transfer N bytes
        // packed four-per-register, MSB-first into the low 32 bits
        // of each successive GPR (high 32 bits zeroed); the register
        // sequence wraps at r31 -> r0. lswi / stswi use NB from the
        // encoding (0 means 32); lswx / stswx use the byte count
        // from XER[57:63] (0 is a no-op for stswx; lswx loads one byte).
        // [PPC-Book1 p:48 s:3.3.6] A load string with RA in the range of registers it loads, RA=0 included, is invalid; for lswx so is RB in the range, or RT=RA or RT=RB.
        // [CBE-Handbook p:254 s:9.5.9] The PPE takes the illegal-instruction interrupt for a load string invalid form.
        // The reference console ends the process on lswi with RA in the
        // range and on lswx with RA or RB in the range.
        PpuInstruction::Lswi { rt, ra, nb } => {
            let n = if nb == 0 { 32usize } else { nb as usize };
            if string_loads_into(rt, n, ra) {
                return ExecuteVerdict::Fault(PpuFault::illegal("lswi"));
            }
            // [PPC-Book1 p:15 s:1.12.2] In 32-bit mode only the low 32 bits of an address sum address storage.
            let base = effective_address(state.ra_or_zero(ra));
            if caching_inhibited(region_views, base, n as u64) {
                return ExecuteVerdict::Fault(PpuFault::alignment(base, false));
            }
            let mut port = LoadPort::new(region_views, store_buf, effects, unit_id);
            string_load(state, &mut port, rt as usize, base, n)
        }
        PpuInstruction::Lswx { rt, ra, rb } => {
            let n = state.xer_tbc() as usize;
            if rt == ra || rt == rb || string_loads_into(rt, n, ra) || string_loads_into(rt, n, rb)
            {
                return ExecuteVerdict::Fault(PpuFault::illegal("lswx"));
            }
            let base = state.ea_x_form(ra, rb);
            // [PPC-Book1 p:48 s:3.3.6] With a zero count the contents of RT are undefined.
            // The reference console loads one byte, as a count of 1 does, and an
            // unmapped EA ends the process (probes ppu_reference_vectors and
            // ppu_may_trap_lswx_n0_null).
            let n = n.max(1);
            if caching_inhibited(region_views, base, n as u64) {
                return ExecuteVerdict::Fault(PpuFault::alignment(base, false));
            }
            let mut port = LoadPort::new(region_views, store_buf, effects, unit_id);
            string_load(state, &mut port, rt as usize, base, n)
        }
        PpuInstruction::Stswi { rs, ra, nb } => {
            let n = if nb == 0 { 32usize } else { nb as usize };
            let base = effective_address(state.ra_or_zero(ra));
            if caching_inhibited(region_views, base, n as u64) {
                return inhibited_store(region_views, base, base, n as u64);
            }
            string_store(state, region_views, store_buf, rs as usize, base, n)
        }
        // [PPC-Book1 p:49 s:3.3.6] A zero count stores no byte, so it has no operand to refuse.
        PpuInstruction::Stswx { rs, ra, rb } => {
            let base = state.ea_x_form(ra, rb);
            let n = state.xer_tbc() as usize;
            if caching_inhibited(region_views, base, n as u64) {
                return inhibited_store(region_views, base, base, n as u64);
            }
            string_store(state, region_views, store_buf, rs as usize, base, n)
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
            let v = store_conditional(state, region_views, store_buf, effects, rs, ea, Width::B8);
            if v == ExecuteVerdict::Continue {
                state.stdcx_executed = state.stdcx_executed.wrapping_add(1);
            }
            v
        }
        PpuInstruction::Stwcx { rs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            let v = store_conditional(state, region_views, store_buf, effects, rs, ea, Width::B4);
            if v == ExecuteVerdict::Continue {
                state.stwcx_executed = state.stwcx_executed.wrapping_add(1);
            }
            v
        }

        // Vector loads / stores
        // [CBE-Handbook p:744 s:A.3] PPE-only VMX additions lvlx/lvrx/stvlx/stvrx: unaligned vector load/store helpers.
        // [CBE-Handbook p:744 s:A.3.3] lvlxl / lvrxl: identical to lvlx / lvrx,
        // with the LRU cache hint that CellGov's no-cache model ignores.
        // [CBE-Handbook p:290 s:9.10] A VSU load or store, left or right, that references caching-inhibited storage takes the alignment interrupt.
        PpuInstruction::Lvlx { vt, ra, rb } | PpuInstruction::Lvlxl { vt, ra, rb } => {
            let addr = state.ea_x_form(ra, rb);
            if caching_inhibited(region_views, addr, 16 - (addr & 15)) {
                return ExecuteVerdict::Fault(PpuFault::alignment(addr, false));
            }
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
            if caching_inhibited(region_views, addr & !15u64, lo) {
                return ExecuteVerdict::Fault(PpuFault::alignment(addr, false));
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
        // [AltiVec-PEM p:6-14 s:6.2] lvebx loads the byte at EA into byte EA[60:63] of VRT.
        // [AltiVec-PEM p:6-16 s:6.2] lvehx loads the halfword at EA & ~1 into its slot of VRT.
        // [AltiVec-PEM p:6-17 s:6.2] lvewx loads the word at EA & ~3 into its slot of VRT.
        // The other lanes are undefined. The `ppu_lve_lanes` capture shows that the console
        // fills them from the aligned quadword at every offset. Each element load is thus an
        // lvx of EA & ~15.
        PpuInstruction::Lvebx { vt, ra, rb }
        | PpuInstruction::Lvehx { vt, ra, rb }
        | PpuInstruction::Lvewx { vt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb) & !15u64;
            let mut port = LoadPort::new(region_views, store_buf, effects, unit_id);
            let val = match read_aligned_16(&mut port, ea) {
                Ok(v) => v,
                Err(ea) => return ExecuteVerdict::MemFault(ea),
            };
            state.set_vr(vt as usize, val);
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:6-29 s:6.2] stvebx: byte at byte-position (EA & 0xF) of VS -> MEM(EA, 1).
        PpuInstruction::Stvebx { vs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            let m = (ea & 0xF) as usize;
            let byte = state.vr[vs as usize].to_be_bytes()[m];
            buffer_store(store_buf, region_views, ea, 1, byte as u64)
        }
        // [AltiVec-PEM p:6-30 s:6.2] stvehx: halfword at lane (EA & 0xE) of VS -> MEM(EA & ~1, 2).
        PpuInstruction::Stvehx { vs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb) & !1u64;
            let m = (ea & 0xF) as usize;
            let bytes = state.vr[vs as usize].to_be_bytes();
            let val = u16::from_be_bytes([bytes[m], bytes[m + 1]]);
            buffer_store(store_buf, region_views, ea, 2, val as u64)
        }
        // [AltiVec-PEM p:6-31 s:6.2] stvewx: word at lane (EA & 0xC) of VS -> MEM(EA & ~3, 4).
        PpuInstruction::Stvewx { vs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb) & !3u64;
            let m = (ea & 0xF) as usize;
            let bytes = state.vr[vs as usize].to_be_bytes();
            let val = u32::from_be_bytes([bytes[m], bytes[m + 1], bytes[m + 2], bytes[m + 3]]);
            buffer_store(store_buf, region_views, ea, 4, val as u64)
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
            if caching_inhibited(region_views, ea, count as u64) {
                return inhibited_store(region_views, ea, ea, count as u64);
            }
            let bytes = state.vr[vs as usize].to_be_bytes();
            let stores = (0..count).map(|i| (ea + i as u64, 1, u64::from(bytes[i])));
            store_prechecked(store_buf, region_views, count, stores, "stvlx byte")
        }
        PpuInstruction::Stvrx { vs, ra, rb } | PpuInstruction::Stvrxl { vs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            let m = (ea & 0xF) as usize;
            if m == 0 {
                return ExecuteVerdict::Continue;
            }
            let aligned = ea & !15u64;
            if caching_inhibited(region_views, aligned, m as u64) {
                return inhibited_store(region_views, ea, aligned, m as u64);
            }
            let bytes = state.vr[vs as usize].to_be_bytes();
            let stores = (0..m).map(|i| (aligned + i as u64, 1, u64::from(bytes[16 - m + i])));
            store_prechecked(store_buf, region_views, m, stores, "stvrx byte")
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
            store_prechecked(store_buf, region_views, 2, halves.into_iter(), "stvx half")
        }

        // Cache control
        // [PPC-Book2 p:20 s:3.2] Data Cache Block set to Zero (dcbz, X-form): zero the block of size n containing EA; treated as a Store.
        PpuInstruction::Dcbz { ra, rb } => {
            let target = state.ea_x_form(ra, rb);
            let ea = target & !(DCBZ_BLOCK_BYTES as u64 - 1);
            // A block that no region holds faults as unmapped first. An
            // untranslated page has no caching attribute.
            if let Err(e) = check_store(region_views, ea, DCBZ_BLOCK_BYTES as u64) {
                return ExecuteVerdict::StoreFault(e);
            }
            // [CBE-Handbook p:147 s:6.1.6.4] A dcbz to caching-inhibited storage takes an alignment interrupt.
            if caching_inhibited(region_views, ea, DCBZ_BLOCK_BYTES as u64) {
                return ExecuteVerdict::Fault(PpuFault::alignment(target, true));
            }
            // Capacity pre-check prevents a partial block from
            // committing on `BufferFull` mid-loop.
            const DCBZ_STORES: usize = DCBZ_BLOCK_BYTES / 8;
            let zeros = (0..DCBZ_STORES).map(|i| (ea + (i as u64) * 8, 8, 0));
            let v = store_prechecked(store_buf, region_views, DCBZ_STORES, zeros, "dcbz");
            if v == ExecuteVerdict::Continue {
                state.dcbz_executed = state.dcbz_executed.wrapping_add(1);
            }
            v
        }
        // [PPC-Book2 p:18 s:3.2.1] icbi is treated as a Load for translation and protection.
        // [PPC-Book2 p:21 s:3.2.2] dcbst and dcbf are treated as a Load.
        // [CBE-Handbook p:249 s:9.5.3] the PPE takes a data storage interrupt for icbi, dcbst and dcbf.
        PpuInstruction::Dcbst { ra, rb }
        | PpuInstruction::Dcbf { ra, rb, .. }
        | PpuInstruction::Icbi { ra, rb } => {
            match check_block_translates(region_views, state.ea_x_form(ra, rb)) {
                Ok(()) => ExecuteVerdict::Continue,
                Err(e) => ExecuteVerdict::MemFault(e),
            }
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
        return ExecuteVerdict::Fault(PpuFault::alignment(ea, false));
    }
    // [CBE-Handbook p:253 s:9.5.8] An lwarx, ldarx, stwcx. or stdcx. operand in caching-inhibited storage takes the alignment interrupt.
    if caching_inhibited(region_views, ea, u64::from(width.bytes())) {
        return ExecuteVerdict::Fault(PpuFault::alignment(ea, false));
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

/// Runs `stwcx.` or `stdcx.`; the caller counts it on `Continue`.
///
/// The local reservation is authoritative: cross-unit clears happen at
/// step start, and the unit's own stores leave it.
#[inline]
fn store_conditional(
    state: &mut PpuState,
    region_views: &[cellgov_mem::RegionView<'_>],
    store_buf: &mut StoreBuffer,
    effects: &[Effect],
    rs: u8,
    ea: u64,
    width: Width,
) -> ExecuteVerdict {
    let size = width.bytes();
    if ea & u64::from(size - 1) != 0 {
        return ExecuteVerdict::Fault(PpuFault::alignment(ea, true));
    }
    // [CBE-Handbook p:249 s:9.5.3] The PPE raises the data storage
    // interrupt whether or not the store would update storage.
    if let Err(e) = check_store(region_views, ea, u64::from(size)) {
        return ExecuteVerdict::StoreFault(e);
    }
    // The alignment interrupt follows a refusal, as `inhibited_store` orders them.
    if caching_inhibited(region_views, ea, u64::from(size)) {
        return ExecuteVerdict::Fault(PpuFault::alignment(ea, true));
    }
    let success = holds_reservation_for(state, ea);
    // [PPC-Book2 p:25 s:3.3.2 Atomic Update Primitives] CR0 = 0b00 || n || XER[SO].
    let so = u8::from(state.xer_so());
    if success {
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
/// check and one address check, so neither a `BufferFull` nor a fault
/// leaves part of the instruction staged; a retry would duplicate the
/// earlier writes.
#[inline]
pub(in crate::exec) fn store_prechecked(
    store_buf: &mut StoreBuffer,
    region_views: &[cellgov_mem::RegionView<'_>],
    count: usize,
    stores: impl Iterator<Item = (u64, u8, u64)> + Clone,
    what: &str,
) -> ExecuteVerdict {
    for (ea, size, _) in stores.clone() {
        if let Err(e) = check_store(region_views, ea, u64::from(size)) {
            return ExecuteVerdict::StoreFault(e);
        }
    }
    if !store_buf.has_capacity_for(count) {
        return ExecuteVerdict::BufferFull;
    }
    for (ea, size, value) in stores {
        let v = buffer_store(store_buf, region_views, ea, size, value);
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
