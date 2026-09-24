//! The memory-instruction dispatch: one match over every load, store, atomic and cache-block form.

use crate::exec::memory_helpers::{
    buffer_conditional_store, buffer_store, load_ze, LoadPort, Width,
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
            let count = 32 - rs as usize;
            if !store_buf.has_capacity_for(count) {
                return ExecuteVerdict::BufferFull;
            }
            let mut ea = state.ea_d_form(ra, imm);
            for r in (rs as usize)..32 {
                let v = buffer_store(store_buf, state, ea, 4, state.gpr[r]);
                debug_assert!(
                    v != ExecuteVerdict::BufferFull,
                    "stmw word store failed after capacity pre-check"
                );
                if v != ExecuteVerdict::Continue {
                    return v;
                }
                ea = ea.wrapping_add(4);
            }
            ExecuteVerdict::Continue
        }

        // [PPC-Book1 p:55 s:3.3.5] String moves transfer N bytes
        // packed four-per-register, MSB-first into the low 32 bits
        // of each successive GPR (high 32 bits zeroed); the register
        // sequence wraps at r31 -> r0. lswi / stswi use NB from the
        // encoding (0 means 32); lswx / stswx use the byte count
        // from XER[57:63] (0 is a no-op).
        PpuInstruction::Lswi { rt, ra, nb } => {
            let n = if nb == 0 { 32usize } else { nb as usize };
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
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
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
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
            if ea & 7 != 0 {
                return ExecuteVerdict::Fault(PpuFault::AlignmentInterrupt(ea));
            }
            let loaded = load_ze(
                &mut LoadPort::new(region_views, store_buf, effects, unit_id),
                ea,
                Width::B8,
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
        // [PPC-Book2 p:25 s:3.3] stwcx./stdcx.: if RESERVE && RESERVE_ADDR == real_addr(EA) then store + CR0 = 0b00||1||XER[SO], else CR0 = 0b00||0||XER[SO]; reservation cleared.
        PpuInstruction::Stdcx { rs, ra, rb } => {
            // Local reservation is authoritative: cross-unit clears
            // happen at step start; the unit's own stores leave it.
            let ea = state.ea_x_form(ra, rb);
            state.stdcx_executed = state.stdcx_executed.wrapping_add(1);
            if ea & 7 != 0 {
                return ExecuteVerdict::Fault(PpuFault::AlignmentInterrupt(ea));
            }
            let success = holds_reservation_for(state, ea);
            // [PPC-Book2 p:25 s:3.3.2 Atomic Update Primitives] CR0 = 0b00 || n || XER[SO].
            let so = u8::from(state.xer_so());
            if success {
                if ByteRange::new(GuestAddr::new(ea), 8).is_none() {
                    return ExecuteVerdict::MemFault(cellgov_mem::MemError::Unmapped(
                        cellgov_mem::FaultContext {
                            addr: ea,
                            nearest_below: None,
                            nearest_above: None,
                        },
                    ));
                }
                // Buffered so the `ConditionalStore` lands in program
                // order (see `StoreBuffer::insert_conditional`); the
                // capacity check comes before CR0 so a full buffer
                // retries with CR0 and the reservation untouched.
                if !store_buf.has_capacity_for(1) {
                    return ExecuteVerdict::BufferFull;
                }
                state.set_cr_field(0, 0b0010 | so);
                let value = state.gpr[rs as usize];
                let staged = buffer_conditional_store(store_buf, ea, 8, value, effects.len());
                debug_assert!(
                    staged != ExecuteVerdict::BufferFull,
                    "stdcx. insert after has_capacity_for(1) passed"
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
        PpuInstruction::Lwarx { rt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            state.lwarx_executed = state.lwarx_executed.wrapping_add(1);
            if ea & 3 != 0 {
                return ExecuteVerdict::Fault(PpuFault::AlignmentInterrupt(ea));
            }
            let loaded = load_ze(
                &mut LoadPort::new(region_views, store_buf, effects, unit_id),
                ea,
                Width::B4,
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
        PpuInstruction::Stwcx { rs, ra, rb } => {
            // See `Stdcx` for the reservation / flush / forward-entry contract.
            let ea = state.ea_x_form(ra, rb);
            state.stwcx_executed = state.stwcx_executed.wrapping_add(1);
            if ea & 3 != 0 {
                return ExecuteVerdict::Fault(PpuFault::AlignmentInterrupt(ea));
            }
            let success = holds_reservation_for(state, ea);
            let so = u8::from(state.xer_so());
            if success {
                if ByteRange::new(GuestAddr::new(ea), 4).is_none() {
                    return ExecuteVerdict::MemFault(cellgov_mem::MemError::Unmapped(
                        cellgov_mem::FaultContext {
                            addr: ea,
                            nearest_below: None,
                            nearest_above: None,
                        },
                    ));
                }
                if !store_buf.has_capacity_for(1) {
                    return ExecuteVerdict::BufferFull;
                }
                state.set_cr_field(0, 0b0010 | so);
                let value32 = state.gpr[rs as usize] as u32;
                let staged =
                    buffer_conditional_store(store_buf, ea, 4, u64::from(value32), effects.len());
                debug_assert!(
                    staged != ExecuteVerdict::BufferFull,
                    "stwcx. insert after has_capacity_for(1) passed"
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

        // Vector loads / stores
        // [CBE-Handbook p:744 s:A.3] PPE-only VMX additions lvlx/lvrx/stvlx/stvrx: unaligned vector load/store helpers.
        PpuInstruction::Lvlx { vt, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let addr = base.wrapping_add(state.gpr[rb as usize]);
            let aligned = addr & !15u64;
            let val = match read_aligned_16(
                &mut LoadPort::new(region_views, store_buf, effects, unit_id),
                aligned,
            ) {
                Ok(v) => v,
                Err(ea) => return ExecuteVerdict::MemFault(ea),
            };
            let shift = ((addr & 15) * 8) as u32;
            state.set_vr(vt as usize, if shift == 0 { val } else { val << shift });
            ExecuteVerdict::Continue
        }
        PpuInstruction::Lvrx { vt, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let addr = base.wrapping_add(state.gpr[rb as usize]);
            let lo = addr & 15;
            // [CBE-Handbook p:744 s:A.3.3 Table A-9] a quadword-aligned
            // lvrx makes no attempt to access storage and delivers
            // zero, so it neither faults on an unmapped line nor
            // records a read of one.
            if lo == 0 {
                state.set_vr(vt as usize, 0);
                return ExecuteVerdict::Continue;
            }
            let val = match read_aligned_16(
                &mut LoadPort::new(region_views, store_buf, effects, unit_id),
                addr & !15u64,
            ) {
                Ok(v) => v,
                Err(ea) => return ExecuteVerdict::MemFault(ea),
            };
            state.set_vr(vt as usize, val >> ((16 - lo) * 8) as u32);
            ExecuteVerdict::Continue
        }
        // [CBE-Handbook p:744 s:A.3.3] lvlxl / lvrxl: identical to lvlx / lvrx,
        // with the LRU cache hint that CellGov's no-cache model ignores.
        PpuInstruction::Lvlxl { vt, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let addr = base.wrapping_add(state.gpr[rb as usize]);
            let aligned = addr & !15u64;
            let val = match read_aligned_16(
                &mut LoadPort::new(region_views, store_buf, effects, unit_id),
                aligned,
            ) {
                Ok(v) => v,
                Err(ea) => return ExecuteVerdict::MemFault(ea),
            };
            let shift = ((addr & 15) * 8) as u32;
            state.set_vr(vt as usize, if shift == 0 { val } else { val << shift });
            ExecuteVerdict::Continue
        }
        PpuInstruction::Lvrxl { vt, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let addr = base.wrapping_add(state.gpr[rb as usize]);
            let lo = addr & 15;
            // [CBE-Handbook p:744 s:A.3.3 Table A-9] see `Lvrx`: the
            // quadword-aligned form touches no storage.
            if lo == 0 {
                state.set_vr(vt as usize, 0);
                return ExecuteVerdict::Continue;
            }
            let val = match read_aligned_16(
                &mut LoadPort::new(region_views, store_buf, effects, unit_id),
                addr & !15u64,
            ) {
                Ok(v) => v,
                Err(ea) => return ExecuteVerdict::MemFault(ea),
            };
            state.set_vr(vt as usize, val >> ((16 - lo) * 8) as u32);
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:6-21 s:6.2] Load Vector Indexed (lvx, X-form): EA = ((RA|0)+(RB)) & ~0xF; MEM(EA,16) -> vT.
        PpuInstruction::Lvx { vt, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]) & !15u64;
            let val = match read_aligned_16(
                &mut LoadPort::new(region_views, store_buf, effects, unit_id),
                ea,
            ) {
                Ok(v) => v,
                Err(ea) => return ExecuteVerdict::MemFault(ea),
            };
            state.set_vr(vt as usize, val);
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:6-23 s:6.2] lvxl: same semantics as lvx; the "Last" suffix is a cache LRU
        // hint that CellGov's no-cache model ignores.
        PpuInstruction::Lvxl { vt, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]) & !15u64;
            let val = match read_aligned_16(
                &mut LoadPort::new(region_views, store_buf, effects, unit_id),
                ea,
            ) {
                Ok(v) => v,
                Err(ea) => return ExecuteVerdict::MemFault(ea),
            };
            state.set_vr(vt as usize, val);
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:6-21 s:6.2] lvsl: VRT[i] = sh + i for i in 0..16, where sh = EA[60:63].
        // Memory is not read; the result is a permute control vector derived from the low 4 bits of EA.
        PpuInstruction::Lvsl { vt, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let sh = (base.wrapping_add(state.gpr[rb as usize]) & 0xF) as u8;
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
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let sh = (base.wrapping_add(state.gpr[rb as usize]) & 0xF) as u8;
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
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]);
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
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]) & !1u64;
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
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]) & !3u64;
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
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]);
            let m = (ea & 0xF) as usize;
            let byte = state.vr[vs as usize].to_be_bytes()[m];
            buffer_store(store_buf, state, ea, 1, byte as u64)
        }
        // [AltiVec-PEM p:6-30 s:6.2] stvehx: halfword at lane (EA & 0xE) of VS -> MEM(EA & ~1, 2).
        PpuInstruction::Stvehx { vs, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]) & !1u64;
            let m = (ea & 0xF) as usize;
            let bytes = state.vr[vs as usize].to_be_bytes();
            let val = u16::from_be_bytes([bytes[m], bytes[m + 1]]);
            buffer_store(store_buf, state, ea, 2, val as u64)
        }
        // [AltiVec-PEM p:6-31 s:6.2] stvewx: word at lane (EA & 0xC) of VS -> MEM(EA & ~3, 4).
        PpuInstruction::Stvewx { vs, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]) & !3u64;
            let m = (ea & 0xF) as usize;
            let bytes = state.vr[vs as usize].to_be_bytes();
            let val = u32::from_be_bytes([bytes[m], bytes[m + 1], bytes[m + 2], bytes[m + 3]]);
            buffer_store(store_buf, state, ea, 4, val as u64)
        }
        // [CBE-Handbook p:744 s:A.3.3] stvlx / stvrx: partial-vector stores. stvlx writes
        // the high `16 - (EA & 0xF)` bytes at EA; stvrx writes the low `EA & 0xF` bytes
        // at the aligned line below EA. Capacity pre-check prevents partial commit on
        // mid-instruction BufferFull (retry would duplicate prior bytes' SharedWriteIntent).
        PpuInstruction::Stvlx { vs, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]);
            let m = (ea & 0xF) as usize;
            let count = 16 - m;
            if !store_buf.has_capacity_for(count) {
                return ExecuteVerdict::BufferFull;
            }
            let bytes = state.vr[vs as usize].to_be_bytes();
            for (i, &b) in bytes.iter().take(count).enumerate() {
                let v = buffer_store(store_buf, state, ea + i as u64, 1, b as u64);
                debug_assert!(
                    v != ExecuteVerdict::BufferFull,
                    "stvlx byte store failed after capacity pre-check"
                );
                if v != ExecuteVerdict::Continue {
                    return v;
                }
            }
            ExecuteVerdict::Continue
        }
        PpuInstruction::Stvrx { vs, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]);
            let m = (ea & 0xF) as usize;
            if m == 0 {
                return ExecuteVerdict::Continue;
            }
            if !store_buf.has_capacity_for(m) {
                return ExecuteVerdict::BufferFull;
            }
            let aligned = ea & !15u64;
            let bytes = state.vr[vs as usize].to_be_bytes();
            for i in 0..m {
                let v = buffer_store(
                    store_buf,
                    state,
                    aligned + i as u64,
                    1,
                    bytes[16 - m + i] as u64,
                );
                debug_assert!(
                    v != ExecuteVerdict::BufferFull,
                    "stvrx byte store failed after capacity pre-check"
                );
                if v != ExecuteVerdict::Continue {
                    return v;
                }
            }
            ExecuteVerdict::Continue
        }
        // [CBE-Handbook p:744 s:A.3.3] stvlxl / stvrxl: identical to stvlx / stvrx; the
        // "Last" suffix is an LRU cache hint CellGov's no-cache model ignores.
        PpuInstruction::Stvlxl { vs, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]);
            let m = (ea & 0xF) as usize;
            let count = 16 - m;
            if !store_buf.has_capacity_for(count) {
                return ExecuteVerdict::BufferFull;
            }
            let bytes = state.vr[vs as usize].to_be_bytes();
            for (i, &b) in bytes.iter().take(count).enumerate() {
                let v = buffer_store(store_buf, state, ea + i as u64, 1, b as u64);
                debug_assert!(
                    v != ExecuteVerdict::BufferFull,
                    "stvlxl byte store failed after capacity pre-check"
                );
                if v != ExecuteVerdict::Continue {
                    return v;
                }
            }
            ExecuteVerdict::Continue
        }
        PpuInstruction::Stvrxl { vs, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]);
            let m = (ea & 0xF) as usize;
            if m == 0 {
                return ExecuteVerdict::Continue;
            }
            if !store_buf.has_capacity_for(m) {
                return ExecuteVerdict::BufferFull;
            }
            let aligned = ea & !15u64;
            let bytes = state.vr[vs as usize].to_be_bytes();
            for i in 0..m {
                let v = buffer_store(
                    store_buf,
                    state,
                    aligned + i as u64,
                    1,
                    bytes[16 - m + i] as u64,
                );
                debug_assert!(
                    v != ExecuteVerdict::BufferFull,
                    "stvrxl byte store failed after capacity pre-check"
                );
                if v != ExecuteVerdict::Continue {
                    return v;
                }
            }
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:6-28 s:6.2] Store Vector Indexed (stvx, X-form): EA = ((RA|0)+(RB)) & ~0xF; vS -> MEM(EA,16).
        PpuInstruction::Stvx { vs, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]) & !15u64;
            // Split into two 8-byte halves so the reservation
            // clear-sweep in buffer_store covers both. Capacity
            // pre-check prevents partial commit on `BufferFull`.
            if !store_buf.has_capacity_for(2) {
                return ExecuteVerdict::BufferFull;
            }
            let bytes = state.vr[vs as usize].to_be_bytes();
            // Direct array indexing of [u8; 16] with constant offsets:
            // bounds are compile-time-evaluable, no runtime panic site.
            let hi = u64::from_be_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]);
            let lo = u64::from_be_bytes([
                bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14],
                bytes[15],
            ]);
            let v1 = buffer_store(store_buf, state, ea, 8, hi);
            debug_assert!(
                v1 != ExecuteVerdict::BufferFull,
                "stvx first half failed after capacity pre-check"
            );
            if v1 != ExecuteVerdict::Continue {
                return v1;
            }
            let v2 = buffer_store(store_buf, state, ea + 8, 8, lo);
            debug_assert!(
                v2 != ExecuteVerdict::BufferFull,
                "stvx second half failed after capacity pre-check"
            );
            v2
        }
        // [AltiVec-PEM p:6-33 s:6.2] stvxl: identical to stvx; the LRU "Last" suffix is a cache
        // hint CellGov's no-cache model ignores.
        PpuInstruction::Stvxl { vs, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]) & !15u64;
            if !store_buf.has_capacity_for(2) {
                return ExecuteVerdict::BufferFull;
            }
            let bytes = state.vr[vs as usize].to_be_bytes();
            let hi = u64::from_be_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]);
            let lo = u64::from_be_bytes([
                bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14],
                bytes[15],
            ]);
            let v1 = buffer_store(store_buf, state, ea, 8, hi);
            debug_assert!(
                v1 != ExecuteVerdict::BufferFull,
                "stvxl first half failed after capacity pre-check"
            );
            if v1 != ExecuteVerdict::Continue {
                return v1;
            }
            let v2 = buffer_store(store_buf, state, ea + 8, 8, lo);
            debug_assert!(
                v2 != ExecuteVerdict::BufferFull,
                "stvxl second half failed after capacity pre-check"
            );
            v2
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
            if !store_buf.has_capacity_for(DCBZ_STORES) {
                return ExecuteVerdict::BufferFull;
            }
            for i in 0..DCBZ_STORES {
                let step = buffer_store(store_buf, state, ea + (i as u64) * 8, 8, 0);
                debug_assert!(
                    step != ExecuteVerdict::BufferFull,
                    "dcbz store unexpectedly failed after capacity check"
                );
                if step != ExecuteVerdict::Continue {
                    return step;
                }
            }
            ExecuteVerdict::Continue
        }

        _ => unreachable!("mem::execute called with non-memory variant"),
    }
}
