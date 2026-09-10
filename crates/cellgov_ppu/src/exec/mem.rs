//! Memory dispatch: integer / atomic / vector / floating-point loads
//! and stores, plus `dcbz`. The scalar loads and stores decode to a
//! [`Scalar`] and run through one [`load`] and one [`store`]. Every
//! path shares the `load_ze` / `load_se` / `buffer_store` /
//! `load_slice` helpers from the parent module so the reservation
//! clear-sweep stays consistent across them.

use crate::exec::memory_helpers::{buffer_store, load_se, load_slice, load_ze, Width};
use crate::exec::{ExecuteVerdict, PpuFault};
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_sync::ReservedLine;

use cellgov_ps3_abi::hw::ppu::DCBZ_BLOCK_BYTES;

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
            return load(state, region_views, store_buf, ea, width, dest, update);
        }
        Some(Scalar::Store(ea, width, src, update)) => {
            return store(state, store_buf, ea, width, src, update);
        }
        None => {}
    }
    match *insn {
        // [PPC-Book1 p:46 s:3.3.5] lmw D-form: for r=RT..31, GPR[r] =
        // zero-extend(MEM(EA,4)); EA += 4. Invalid form if RA is in
        // (RT..=31) or RA==0; the load would otherwise overwrite RA
        // mid-loop, silently corrupting the base register.
        PpuInstruction::Lmw { rt, ra, imm } => {
            debug_assert!(
                ra != 0 && (ra as usize) < (rt as usize),
                "lmw invalid form: RA={} must be non-zero and outside [{}..=31]; \
                 a guest encoding with RA in the load range would silently \
                 corrupt RA mid-loop",
                ra,
                rt
            );
            let mut ea = state.ea_d_form(ra, imm);
            for r in (rt as usize)..32 {
                match load_ze(region_views, store_buf, ea, Width::B4) {
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
                debug_assert_eq!(
                    v,
                    ExecuteVerdict::Continue,
                    "stmw word store failed after capacity pre-check"
                );
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
            string_load(state, region_views, store_buf, rt as usize, base, n)
        }
        PpuInstruction::Lswx { rt, ra, rb } => {
            let base = state.ea_x_form(ra, rb);
            let n = state.xer_tbc() as usize;
            string_load(state, region_views, store_buf, rt as usize, base, n)
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
            match load_ze(region_views, store_buf, ea, Width::B8) {
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
            let success = match state.reservation() {
                Some(line) => line.addr() == ReservedLine::containing(ea).addr(),
                None => false,
            };
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
                let staged = store_buf.insert_conditional(ea, 8, value as u128, effects.len());
                debug_assert!(staged, "stdcx. insert after has_capacity_for(1) passed");
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
            match load_ze(region_views, store_buf, ea, Width::B4) {
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
            let success = match state.reservation() {
                Some(line) => line.addr() == ReservedLine::containing(ea).addr(),
                None => false,
            };
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
                let staged = store_buf.insert_conditional(ea, 4, value32 as u128, effects.len());
                debug_assert!(staged, "stwcx. insert after has_capacity_for(1) passed");
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
            let val = match read_aligned_16(aligned, region_views, store_buf) {
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
            let aligned = addr & !15u64;
            let val = match read_aligned_16(aligned, region_views, store_buf) {
                Ok(v) => v,
                Err(ea) => return ExecuteVerdict::MemFault(ea),
            };
            let lo = addr & 15;
            state.set_vr(
                vt as usize,
                if lo == 0 {
                    0
                } else {
                    val >> ((16 - lo) * 8) as u32
                },
            );
            ExecuteVerdict::Continue
        }
        // [CBE-Handbook p:744 s:A.3.3] lvlxl / lvrxl: identical to lvlx / lvrx,
        // with the LRU cache hint that CellGov's no-cache model ignores.
        PpuInstruction::Lvlxl { vt, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let addr = base.wrapping_add(state.gpr[rb as usize]);
            let aligned = addr & !15u64;
            let val = match read_aligned_16(aligned, region_views, store_buf) {
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
            let aligned = addr & !15u64;
            let val = match read_aligned_16(aligned, region_views, store_buf) {
                Ok(v) => v,
                Err(ea) => return ExecuteVerdict::MemFault(ea),
            };
            let lo = addr & 15;
            state.set_vr(
                vt as usize,
                if lo == 0 {
                    0
                } else {
                    val >> ((16 - lo) * 8) as u32
                },
            );
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:6-21 s:6.2] Load Vector Indexed (lvx, X-form): EA = ((RA|0)+(RB)) & ~0xF; MEM(EA,16) -> vT.
        PpuInstruction::Lvx { vt, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]) & !15u64;
            let val = match read_aligned_16(ea, region_views, store_buf) {
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
            let val = match read_aligned_16(ea, region_views, store_buf) {
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
            let byte = match load_ze(region_views, store_buf, ea, Width::B1) {
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
            let val = match load_ze(region_views, store_buf, ea, Width::B2) {
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
            let val = match load_ze(region_views, store_buf, ea, Width::B4) {
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
                debug_assert_eq!(
                    v,
                    ExecuteVerdict::Continue,
                    "stvlx byte store failed after capacity pre-check"
                );
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
                debug_assert_eq!(
                    v,
                    ExecuteVerdict::Continue,
                    "stvrx byte store failed after capacity pre-check"
                );
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
                debug_assert_eq!(
                    v,
                    ExecuteVerdict::Continue,
                    "stvlxl byte store failed after capacity pre-check"
                );
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
                debug_assert_eq!(
                    v,
                    ExecuteVerdict::Continue,
                    "stvrxl byte store failed after capacity pre-check"
                );
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
            debug_assert_eq!(
                v1,
                ExecuteVerdict::Continue,
                "stvx first half failed after capacity pre-check"
            );
            let v2 = buffer_store(store_buf, state, ea + 8, 8, lo);
            debug_assert_eq!(
                v2,
                ExecuteVerdict::Continue,
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
            debug_assert_eq!(
                v1,
                ExecuteVerdict::Continue,
                "stvxl first half failed after capacity pre-check"
            );
            let v2 = buffer_store(store_buf, state, ea + 8, 8, lo);
            debug_assert_eq!(
                v2,
                ExecuteVerdict::Continue,
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
                debug_assert_eq!(
                    step,
                    ExecuteVerdict::Continue,
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

/// The effective-address form of a scalar load or store.
#[derive(Clone, Copy)]
enum Ea {
    /// `(RA|0) + EXTS(D)`; the decoder clears the low two bits of a
    /// DS-form offset.
    D(u8, i16),
    /// `(RA|0) + (RB)`.
    X(u8, u8),
}

impl Ea {
    #[inline]
    fn resolve(self, state: &PpuState) -> u64 {
        match self {
            Ea::D(ra, imm) => state.ea_d_form(ra, imm),
            Ea::X(ra, rb) => state.ea_x_form(ra, rb),
        }
    }

    /// The base register an update form writes EA back to.
    fn ra(self) -> u8 {
        match self {
            Ea::D(ra, _) | Ea::X(ra, _) => ra,
        }
    }
}

/// Where a scalar load lands, and how it widens the bytes on the way.
#[derive(Clone, Copy)]
enum Dest {
    /// Zero-extended into a GPR.
    Zero(u8),
    /// Sign-extended into a GPR.
    Sign(u8),
    /// Byte-reversed, then zero-extended into a GPR.
    Reversed(u8),
    /// Into an FPR: a word goes through `DOUBLE`, a doubleword lands
    /// verbatim.
    Fpr(u8),
}

/// What a scalar store writes; the store buffer keeps the low `width`
/// bytes of the value.
#[derive(Clone, Copy)]
enum Src {
    Gpr(u8),
    /// A GPR, byte-reversed within the stored width.
    Reversed(u8),
    /// An FPR: a word goes through `SINGLE`, a doubleword passes
    /// verbatim.
    Fpr(u8),
    /// An FPR's raw bits with no conversion at any width.
    FprBits(u8),
}

/// Whether the instruction writes EA to RA once the access succeeds.
#[derive(Clone, Copy)]
enum Update {
    No,
    /// Write EA to RA; the mnemonic names the instruction in the
    /// invalid-form assertion.
    Ra(&'static str),
}

/// A scalar load or store, split on the four axes its forms differ by:
///
/// - effective-address form
/// - width
/// - extension / register file
/// - base-register update
#[derive(Clone, Copy)]
enum Scalar {
    Load(Ea, Width, Dest, Update),
    Store(Ea, Width, Src, Update),
}

/// Decode the scalar load / store forms; `None` for every other
/// memory instruction.
#[inline]
fn scalar(insn: &PpuInstruction) -> Option<Scalar> {
    use Ea::{D, X};
    use Scalar::{Load, Store};
    use Update::{No, Ra};
    use Width::{B1, B2, B4, B8};
    Some(match *insn {
        // Integer loads
        // [PPC-Book1 p:34 s:3.3] Load Byte and Zero (lbz, D-form): byte at EA -> RT[56:63], RT[0:55]=0.
        // [PPC-Book1 p:35 s:3.3] Load Halfword and Zero (lhz, D-form): zero-extend halfword to RT.
        // [PPC-Book1 p:36 s:3.3] Load Halfword Algebraic (lha, D-form): sign-extend halfword to RT.
        // [PPC-Book1 p:38 s:3.3] Load Word Algebraic (lwa, DS-form): sign-extend word to RT.
        // [PPC-Book1 p:39 s:3.3] Load Doubleword (ld, DS-form): MEM(EA,8) -> RT.
        PpuInstruction::Lwz { rt, ra, imm } => Load(D(ra, imm), B4, Dest::Zero(rt), No),
        PpuInstruction::Lbz { rt, ra, imm } => Load(D(ra, imm), B1, Dest::Zero(rt), No),
        PpuInstruction::Lhz { rt, ra, imm } => Load(D(ra, imm), B2, Dest::Zero(rt), No),
        PpuInstruction::Lha { rt, ra, imm } => Load(D(ra, imm), B2, Dest::Sign(rt), No),
        // [PPC-Book1 p:36 s:3.3] lhau D-form: sign-extend halfword to RT; RA = EA. Requires RA != 0 && RA != RT.
        PpuInstruction::Lhau { rt, ra, imm } => Load(D(ra, imm), B2, Dest::Sign(rt), Ra("lhau")),
        PpuInstruction::Lwzu { rt, ra, imm } => Load(D(ra, imm), B4, Dest::Zero(rt), Ra("lwzu")),
        PpuInstruction::Lbzu { rt, ra, imm } => Load(D(ra, imm), B1, Dest::Zero(rt), Ra("lbzu")),
        PpuInstruction::Lhzu { rt, ra, imm } => Load(D(ra, imm), B2, Dest::Zero(rt), Ra("lhzu")),
        PpuInstruction::Ldu { rt, ra, imm } => Load(D(ra, imm), B8, Dest::Zero(rt), Ra("ldu")),
        PpuInstruction::Ld { rt, ra, imm } => Load(D(ra, imm), B8, Dest::Zero(rt), No),
        PpuInstruction::Lwa { rt, ra, imm } => Load(D(ra, imm), B4, Dest::Sign(rt), No),
        // [PPC-Book1 p:34 s:3.3] X-form indexed load variants (lbzx/lhzx/lwzx/ldx): EA = (RA|0)+(RB).
        PpuInstruction::Lwzx { rt, ra, rb } => Load(X(ra, rb), B4, Dest::Zero(rt), No),
        PpuInstruction::Lbzx { rt, ra, rb } => Load(X(ra, rb), B1, Dest::Zero(rt), No),
        PpuInstruction::Ldx { rt, ra, rb } => Load(X(ra, rb), B8, Dest::Zero(rt), No),
        PpuInstruction::Lhzx { rt, ra, rb } => Load(X(ra, rb), B2, Dest::Zero(rt), No),
        // [PPC-Book1 p:34 s:3.3.1] X-form indexed loads with update: EA = (RA|0)+(RB), then RA = EA.
        PpuInstruction::Lwzux { rt, ra, rb } => Load(X(ra, rb), B4, Dest::Zero(rt), Ra("lwzux")),
        PpuInstruction::Lbzux { rt, ra, rb } => Load(X(ra, rb), B1, Dest::Zero(rt), Ra("lbzux")),
        PpuInstruction::Lhzux { rt, ra, rb } => Load(X(ra, rb), B2, Dest::Zero(rt), Ra("lhzux")),
        PpuInstruction::Ldux { rt, ra, rb } => Load(X(ra, rb), B8, Dest::Zero(rt), Ra("ldux")),
        // [PPC-Book1 p:36 s:3.3] lhax / lhaux: load halfword algebraic (sign-extend 16->64).
        PpuInstruction::Lhax { rt, ra, rb } => Load(X(ra, rb), B2, Dest::Sign(rt), No),
        PpuInstruction::Lhaux { rt, ra, rb } => Load(X(ra, rb), B2, Dest::Sign(rt), Ra("lhaux")),
        // [PPC-Book1 p:38 s:3.3] lwax / lwaux: load word algebraic (sign-extend 32->64).
        PpuInstruction::Lwax { rt, ra, rb } => Load(X(ra, rb), B4, Dest::Sign(rt), No),
        PpuInstruction::Lwaux { rt, ra, rb } => Load(X(ra, rb), B4, Dest::Sign(rt), Ra("lwaux")),

        // Integer stores
        // [PPC-Book1 p:40 s:3.3.3] Store Byte (stb, D-form): RS[56:63] -> MEM(EA,1).
        // [PPC-Book1 p:41 s:3.3.3] Store Halfword (sth, D-form): RS[48:63] -> MEM(EA,2).
        // [PPC-Book1 p:42 s:3.3.3] Store Word (stw/stwx/stwu, D/X-form): RS[32:63] -> MEM(EA,4).
        // [PPC-Book1 p:43 s:3.3.3] Store Doubleword (std/stdx, DS/X-form): RS -> MEM(EA,8).
        PpuInstruction::Stw { rs, ra, imm } => Store(D(ra, imm), B4, Src::Gpr(rs), No),
        PpuInstruction::Stb { rs, ra, imm } => Store(D(ra, imm), B1, Src::Gpr(rs), No),
        PpuInstruction::Stbu { rs, ra, imm } => Store(D(ra, imm), B1, Src::Gpr(rs), Ra("stbu")),
        PpuInstruction::Sth { rs, ra, imm } => Store(D(ra, imm), B2, Src::Gpr(rs), No),
        PpuInstruction::Sthu { rs, ra, imm } => Store(D(ra, imm), B2, Src::Gpr(rs), Ra("sthu")),
        PpuInstruction::Std { rs, ra, imm } => Store(D(ra, imm), B8, Src::Gpr(rs), No),
        PpuInstruction::Stwu { rs, ra, imm } => Store(D(ra, imm), B4, Src::Gpr(rs), Ra("stwu")),
        PpuInstruction::Stdu { rs, ra, imm } => Store(D(ra, imm), B8, Src::Gpr(rs), Ra("stdu")),
        PpuInstruction::Stwx { rs, ra, rb } => Store(X(ra, rb), B4, Src::Gpr(rs), No),
        PpuInstruction::Stdx { rs, ra, rb } => Store(X(ra, rb), B8, Src::Gpr(rs), No),
        PpuInstruction::Stdux { rs, ra, rb } => Store(X(ra, rb), B8, Src::Gpr(rs), Ra("stdux")),
        PpuInstruction::Stbx { rs, ra, rb } => Store(X(ra, rb), B1, Src::Gpr(rs), No),
        // [PPC-Book1 p:41 s:3.3.3] sthx X-form: low halfword of RS -> MEM(EA, 2).
        PpuInstruction::Sthx { rs, ra, rb } => Store(X(ra, rb), B2, Src::Gpr(rs), No),
        // [PPC-Book1 p:41 s:3.3.3] sthux X-form with update; RA != 0.
        PpuInstruction::Sthux { rs, ra, rb } => Store(X(ra, rb), B2, Src::Gpr(rs), Ra("sthux")),
        // [PPC-Book1 p:42 s:3.3.3] stwux X-form with update; RA != 0.
        PpuInstruction::Stwux { rs, ra, rb } => Store(X(ra, rb), B4, Src::Gpr(rs), Ra("stwux")),
        // [PPC-Book1 p:40 s:3.3.3] stbux X-form with update; RA != 0.
        PpuInstruction::Stbux { rs, ra, rb } => Store(X(ra, rb), B1, Src::Gpr(rs), Ra("stbux")),

        // Byte-reverse indexed loads and stores
        // [PPC-Book1 p:50 s:3.3.4] lwbrx / lhbrx: load size N, low N bytes byte-reversed, zero-extended into RT.
        // [PPC-Book1 p:51 s:3.3.4] ldbrx / stwbrx / sthbrx: doubleword load / word + halfword store with byte reversal.
        // [CBE-Handbook p:734 s:A.2.1] sdbrx (CG name): low-64 byte-reverse store.
        PpuInstruction::Ldbrx { rt, ra, rb } => Load(X(ra, rb), B8, Dest::Reversed(rt), No),
        PpuInstruction::Lwbrx { rt, ra, rb } => Load(X(ra, rb), B4, Dest::Reversed(rt), No),
        PpuInstruction::Lhbrx { rt, ra, rb } => Load(X(ra, rb), B2, Dest::Reversed(rt), No),
        PpuInstruction::Sdbrx { rs, ra, rb } => Store(X(ra, rb), B8, Src::Reversed(rs), No),
        PpuInstruction::Stwbrx { rs, ra, rb } => Store(X(ra, rb), B4, Src::Reversed(rs), No),
        PpuInstruction::Sthbrx { rs, ra, rb } => Store(X(ra, rb), B2, Src::Reversed(rs), No),

        // Floating-point loads / stores
        // [PPC-Book1 p:104 s:4.6] Load Floating-Point Single (lfs, D-form): single -> double via DOUBLE() into FRT.
        // [PPC-Book1 p:105 s:4.6] Load Floating-Point Double (lfd, D-form): MEM(EA,8) -> FRT.
        // [PPC-Book1 p:107 s:4.6] Store Floating-Point Single (stfs, D-form): SINGLE(FRS) -> MEM(EA,4).
        // [PPC-Book1 p:108 s:4.6] Store Floating-Point Double (stfd, D-form): FRS -> MEM(EA,8).
        PpuInstruction::Lfs { frt, ra, imm } => Load(D(ra, imm), B4, Dest::Fpr(frt), No),
        // [PPC-Book1 p:104 s:4.6.2] lfsu D-form: lfs with EA written to RA. Requires RA != 0.
        PpuInstruction::Lfsu { frt, ra, imm } => Load(D(ra, imm), B4, Dest::Fpr(frt), Ra("lfsu")),
        PpuInstruction::Lfd { frt, ra, imm } => Load(D(ra, imm), B8, Dest::Fpr(frt), No),
        // [PPC-Book1 p:105 s:4.6.2] lfdu D-form: lfd with EA written to RA. Requires RA != 0.
        PpuInstruction::Lfdu { frt, ra, imm } => Load(D(ra, imm), B8, Dest::Fpr(frt), Ra("lfdu")),
        PpuInstruction::Stfs { frs, ra, imm } => Store(D(ra, imm), B4, Src::Fpr(frs), No),
        PpuInstruction::Stfd { frs, ra, imm } => Store(D(ra, imm), B8, Src::Fpr(frs), No),
        PpuInstruction::Stfsu { frs, ra, imm } => Store(D(ra, imm), B4, Src::Fpr(frs), Ra("stfsu")),
        PpuInstruction::Stfdu { frs, ra, imm } => Store(D(ra, imm), B8, Src::Fpr(frs), Ra("stfdu")),
        // Unlike stfs, stfiwx stores the low 32 FPR bits verbatim
        // (no round-convert to single precision).
        // [PPC-Book1 p:109 s:4.6] Store Floating-Point as Integer Word Indexed (stfiwx): FRS[32:63] -> MEM(EA,4) without conversion.
        PpuInstruction::Stfiwx { frs, ra, rb } => Store(X(ra, rb), B4, Src::FprBits(frs), No),
        // X-form FP indexed loads / stores. EA = (RA == 0 ? 0 : GPR[RA]) + GPR[RB].
        // The `u` (update) variants write EA back into GPR[RA] iff the
        // memory access succeeded, matching the D-form Stfsu/Stfdu policy.
        // [PPC-Book1 p:104 s:4.6] lfsx / lfsux: X-form single-precision load with DOUBLE() conversion.
        // [PPC-Book1 p:105 s:4.6] lfdx / lfdux: X-form double-precision load.
        // [PPC-Book1 p:107 s:4.6] stfsx / stfsux: X-form single-precision store via SINGLE() conversion.
        // [PPC-Book1 p:108 s:4.6] stfdx / stfdux: X-form double-precision store.
        PpuInstruction::Lfsx { frt, ra, rb } => Load(X(ra, rb), B4, Dest::Fpr(frt), No),
        PpuInstruction::Lfsux { frt, ra, rb } => Load(X(ra, rb), B4, Dest::Fpr(frt), Ra("lfsux")),
        PpuInstruction::Lfdx { frt, ra, rb } => Load(X(ra, rb), B8, Dest::Fpr(frt), No),
        PpuInstruction::Lfdux { frt, ra, rb } => Load(X(ra, rb), B8, Dest::Fpr(frt), Ra("lfdux")),
        PpuInstruction::Stfsx { frs, ra, rb } => Store(X(ra, rb), B4, Src::Fpr(frs), No),
        PpuInstruction::Stfsux { frs, ra, rb } => Store(X(ra, rb), B4, Src::Fpr(frs), Ra("stfsux")),
        PpuInstruction::Stfdx { frs, ra, rb } => Store(X(ra, rb), B8, Src::Fpr(frs), No),
        PpuInstruction::Stfdux { frs, ra, rb } => Store(X(ra, rb), B8, Src::Fpr(frs), Ra("stfdux")),
        _ => return None,
    })
}

/// Runs one scalar load, then writes EA to RA for an update form.
///
/// The writeback follows a successful load only, so a fault leaves RA
/// intact.
#[inline]
fn load(
    state: &mut PpuState,
    region_views: &[cellgov_mem::RegionView<'_>],
    store_buf: &StoreBuffer,
    ea: Ea,
    width: Width,
    dest: Dest,
    update: Update,
) -> ExecuteVerdict {
    if let Update::Ra(insn) = update {
        let ra = ea.ra();
        match dest {
            // [PPC-Book1 p:33 s:3.3.2] Fixed-point load with update: invalid form when RA=0 or RA=RT.
            Dest::Zero(rt) | Dest::Sign(rt) | Dest::Reversed(rt) => {
                debug_assert!(ra != 0 && ra != rt, "{insn} invalid form: RA={ra}, RT={rt}");
            }
            // [PPC-Book1 p:104 s:4.6.2] Floating-point load with update: the only invalid form is RA=0.
            // FRT indexes a different register file, so RA=FRT is a valid encoding.
            Dest::Fpr(_) => debug_assert!(ra != 0, "{insn} invalid form: RA=0"),
        }
    }
    let addr = ea.resolve(state);
    let loaded = match dest {
        Dest::Sign(_) => load_se(region_views, store_buf, addr, width),
        Dest::Zero(_) | Dest::Reversed(_) | Dest::Fpr(_) => {
            load_ze(region_views, store_buf, addr, width)
        }
    };
    let val = match loaded {
        Ok(val) => val,
        Err(e) => return ExecuteVerdict::MemFault(e),
    };
    match dest {
        Dest::Zero(rt) | Dest::Sign(rt) => state.set_gpr(rt as usize, val),
        Dest::Reversed(rt) => state.set_gpr(rt as usize, swap_low_bytes(val, width)),
        Dest::Fpr(frt) => state.set_fpr(
            frt as usize,
            match width {
                Width::B4 => double_word(val as u32),
                Width::B1 | Width::B2 | Width::B8 => val,
            },
        ),
    }
    if let Update::Ra(_) = update {
        state.set_gpr(ea.ra() as usize, addr);
    }
    ExecuteVerdict::Continue
}

/// Stages one scalar store, then writes EA to RA for an update form.
///
/// The writeback follows a staged store only, so a `BufferFull`
/// verdict leaves RA intact for the retry.
#[inline]
fn store(
    state: &mut PpuState,
    store_buf: &mut StoreBuffer,
    ea: Ea,
    width: Width,
    src: Src,
    update: Update,
) -> ExecuteVerdict {
    if let Update::Ra(insn) = update {
        // [PPC-Book1 p:40 s:3.3.3] Store with update: invalid form when RA=0.
        debug_assert!(ea.ra() != 0, "{insn} invalid form: RA=0");
    }
    let addr = ea.resolve(state);
    let value = match src {
        Src::Gpr(rs) => state.gpr[rs as usize],
        Src::Reversed(rs) => swap_low_bytes(state.gpr[rs as usize], width),
        Src::Fpr(frs) => match width {
            Width::B4 => single_frs(state.fpr[frs as usize]) as u64,
            Width::B1 | Width::B2 | Width::B8 => state.fpr[frs as usize],
        },
        Src::FprBits(frs) => state.fpr[frs as usize],
    };
    let verdict = buffer_store(store_buf, state, addr, width.bytes(), value);
    if let Update::Ra(_) = update {
        if verdict.allows_writeback() {
            state.set_gpr(ea.ra() as usize, addr);
        }
    }
    verdict
}

/// Byte-reverse the low `width` bytes of `val` and zero the bytes
/// above them.
#[inline]
fn swap_low_bytes(val: u64, width: Width) -> u64 {
    match width {
        Width::B1 => val as u8 as u64,
        Width::B2 => (val as u16).swap_bytes() as u64,
        Width::B4 => (val as u32).swap_bytes() as u64,
        Width::B8 => val.swap_bytes(),
    }
}

/// Resolve a 16-byte aligned vector-line read with store-buffer overlay.
///
/// # Errors
///
/// Returns an `Unmapped` `MemError` when no region view covers the line.
fn read_aligned_16(
    aligned: u64,
    region_views: &[cellgov_mem::RegionView<'_>],
    store_buf: &StoreBuffer,
) -> Result<u128, cellgov_mem::MemError> {
    if let Some(v) = store_buf.forward(aligned, 16) {
        return Ok(v);
    }
    let slice = match load_slice(region_views, aligned, 16) {
        Some(s) => s,
        None => {
            return Err(cellgov_mem::MemError::Unmapped(cellgov_mem::FaultContext {
                addr: aligned,
                nearest_below: None,
                nearest_above: None,
            }));
        }
    };
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(slice);
    store_buf.overlay_range(aligned, &mut bytes);
    Ok(u128::from_be_bytes(bytes))
}

#[inline]
#[track_caller]
// [PPC-Book1 p:103 s:4.6.2] DOUBLE(WORD): single-precision to double-precision conversion pseudocode (normalized / denormalized / Zero / Infinity / NaN branches).
/// PPC `DOUBLE(WORD)`: 32-bit single -> 64-bit double; preserves NaN
/// payloads bit-exactly so SNaNs survive stfsx -> lfsx round-trips.
fn double_word(w: u32) -> u64 {
    let exp = (w >> 23) & 0xFF;
    let frac23 = w & 0x007F_FFFF;
    if exp == 0xFF && frac23 != 0 {
        // NaN: WORD2:31 || 0^29 fills FRT5:63; FRT1:4 inherit WORD1.
        let sign = ((w >> 31) & 1) as u64;
        let frac52 = (frac23 as u64) << 29;
        return (sign << 63) | (0x7FFu64 << 52) | frac52;
    }
    (f32::from_bits(w) as f64).to_bits()
}

// [PPC-Book1 p:106 s:4.6.3] SINGLE(FRS): double-precision to single-precision conversion pseudocode (No Denormalization Required vs Denormalization Required branches).
/// PPC `SINGLE(FRS)`: 64-bit double -> 32-bit single; preserves NaN
/// payloads bit-exactly.
fn single_frs(d: u64) -> u32 {
    let exp = ((d >> 52) & 0x7FF) as u32;
    let frac52 = d & 0x000F_FFFF_FFFF_FFFF;
    if exp == 0x7FF && frac52 != 0 {
        // NaN: WORD0:1 <- FRS0:1 (sign + first exp bit = 1);
        // WORD2:31 <- FRS5:34 (rest of exp = 1s + top 23 fraction bits).
        let sign = ((d >> 63) & 1) as u32;
        let frac23 = ((d >> 29) & 0x007F_FFFF) as u32;
        return (sign << 31) | (0xFFu32 << 23) | frac23;
    }
    (f64::from_bits(d) as f32).to_bits()
}

/// `lswi` / `lswx` core: read `n` bytes from `base` and pack
/// MSB-first four-per-register into successive GPRs starting at
/// `rt_start`, wrapping at r31 -> r0. Zero-length is a no-op.
fn string_load(
    state: &mut PpuState,
    region_views: &[cellgov_mem::RegionView<'_>],
    store_buf: &StoreBuffer,
    rt_start: usize,
    base: u64,
    n: usize,
) -> ExecuteVerdict {
    if n == 0 {
        return ExecuteVerdict::Continue;
    }
    let mut reg = rt_start % 32;
    let mut byte_idx = 0usize;
    state.set_gpr(reg, 0);
    for i in 0..n {
        let ea = base.wrapping_add(i as u64);
        let byte = match load_ze(region_views, store_buf, ea, Width::B1) {
            Ok(v) => v as u8,
            Err(e) => return ExecuteVerdict::MemFault(e),
        };
        let shift = (3 - byte_idx) * 8;
        state.set_gpr(reg, state.gpr[reg] | ((byte as u64) << shift));
        byte_idx += 1;
        if byte_idx == 4 && i + 1 < n {
            byte_idx = 0;
            reg = (reg + 1) % 32;
            state.set_gpr(reg, 0);
        }
    }
    ExecuteVerdict::Continue
}

/// `stswi` / `stswx` core: store `n` bytes from `base`, extracting
/// MSB-first four-per-register from successive GPRs starting at
/// `rs_start` and wrapping at r31 -> r0. Capacity pre-check
/// prevents partial commit on `BufferFull`.
fn string_store(
    state: &mut PpuState,
    store_buf: &mut StoreBuffer,
    rs_start: usize,
    base: u64,
    n: usize,
) -> ExecuteVerdict {
    if n == 0 {
        return ExecuteVerdict::Continue;
    }
    if !store_buf.has_capacity_for(n) {
        return ExecuteVerdict::BufferFull;
    }
    let mut reg = rs_start % 32;
    let mut byte_idx = 0usize;
    for i in 0..n {
        let shift = (3 - byte_idx) * 8;
        let byte = ((state.gpr[reg] >> shift) & 0xFF) as u8;
        let v = buffer_store(
            store_buf,
            state,
            base.wrapping_add(i as u64),
            1,
            byte as u64,
        );
        debug_assert_eq!(
            v,
            ExecuteVerdict::Continue,
            "string-store byte failed after capacity pre-check"
        );
        byte_idx += 1;
        if byte_idx == 4 {
            byte_idx = 0;
            reg = (reg + 1) % 32;
        }
    }
    ExecuteVerdict::Continue
}

#[cfg(test)]
#[path = "tests/mem_tests.rs"]
mod tests;
