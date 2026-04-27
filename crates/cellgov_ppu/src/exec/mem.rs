//! Memory dispatch: integer / atomic / vector / floating-point loads
//! and stores, plus `dcbz`. All paths share the `load_ze` / `load_se`
//! / `buffer_store` / `load_slice` vocabulary in the parent module so
//! the reservation clear-sweep stays consistent across them.

use crate::exec::{buffer_store, load_se, load_slice, load_ze, ExecuteVerdict};
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_sync::ReservedLine;
use cellgov_time::GuestTicks;

/// Data cache block size on the Cell PPU. Book II Sec. 3.2.2 defines
/// the `dcbz` block as the implementation's data cache line; Cell
/// PPU is 128 bytes.
const DCBZ_BLOCK_BYTES: usize = 128;

pub(crate) fn execute(
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
            debug_assert_load_with_update("lwzu", ra, rt);
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
            debug_assert_load_with_update("lbzu", ra, rt);
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
            debug_assert_load_with_update("lhzu", ra, rt);
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
            debug_assert_load_with_update("ldu", ra, rt);
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
        PpuInstruction::Lwa { rt, ra, imm } => {
            // Load word algebraic: sign-extend the 32-bit value
            // into the 64-bit RT.
            let ea = state.ea_d_form(ra, imm);
            match load_se(region_views, store_buf, ea, 4) {
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
        PpuInstruction::Stbu { rs, ra, imm } => {
            debug_assert_store_with_update("stbu", ra);
            let ea = state.ea_d_form(ra, imm);
            let v = buffer_store(store_buf, state, ea, 1, state.gpr[rs as usize]);
            if v == ExecuteVerdict::Continue {
                state.gpr[ra as usize] = ea;
            }
            v
        }
        PpuInstruction::Sth { rs, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            buffer_store(store_buf, state, ea, 2, state.gpr[rs as usize])
        }
        PpuInstruction::Sthu { rs, ra, imm } => {
            debug_assert_store_with_update("sthu", ra);
            let ea = state.ea_d_form(ra, imm);
            let v = buffer_store(store_buf, state, ea, 2, state.gpr[rs as usize]);
            if v == ExecuteVerdict::Continue {
                state.gpr[ra as usize] = ea;
            }
            v
        }
        PpuInstruction::Std { rs, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            buffer_store(store_buf, state, ea, 8, state.gpr[rs as usize])
        }
        PpuInstruction::Stwu { rs, ra, imm } => {
            debug_assert_store_with_update("stwu", ra);
            let ea = state.ea_d_form(ra, imm);
            let v = buffer_store(store_buf, state, ea, 4, state.gpr[rs as usize]);
            if v == ExecuteVerdict::Continue {
                state.gpr[ra as usize] = ea;
            }
            v
        }
        PpuInstruction::Stdu { rs, ra, imm } => {
            debug_assert_store_with_update("stdu", ra);
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
            debug_assert_store_with_update("stdux", ra);
            let ea = state.ea_x_form(ra, rb);
            let verdict = buffer_store(store_buf, state, ea, 8, state.gpr[rs as usize]);
            if matches!(verdict, ExecuteVerdict::Continue) {
                state.gpr[ra as usize] = ea;
            }
            verdict
        }
        PpuInstruction::Stbx { rs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            buffer_store(store_buf, state, ea, 1, state.gpr[rs as usize])
        }

        // Atomic load-reserve / store-conditional
        PpuInstruction::Ldarx { rt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            debug_assert!(
                ea & 7 == 0,
                "ldarx EA=0x{ea:x} not 8-byte aligned (alignment interrupt on real PPE)"
            );
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
            debug_assert!(
                ea & 7 == 0,
                "stdcx EA=0x{ea:x} not 8-byte aligned (alignment interrupt on real PPE)"
            );
            let success = match state.reservation {
                Some(line) => line.addr() == ReservedLine::containing(ea).addr(),
                None => false,
            };
            // Book II 3.3.2: CR0 = 0b00 || n || XER[SO]. The SO bit
            // is sticky and must be reflected in every dot-form
            // result.
            let so = u8::from(state.xer_so());
            if success {
                state.set_cr_field(0, 0b0010 | so);
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
                state.set_cr_field(0, so);
            }
            state.reservation = None;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Lwarx { rt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            debug_assert!(
                ea & 3 == 0,
                "lwarx EA=0x{ea:x} not 4-byte aligned (alignment interrupt on real PPE)"
            );
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
            debug_assert!(
                ea & 3 == 0,
                "stwcx EA=0x{ea:x} not 4-byte aligned (alignment interrupt on real PPE)"
            );
            let success = match state.reservation {
                Some(line) => line.addr() == ReservedLine::containing(ea).addr(),
                None => false,
            };
            let so = u8::from(state.xer_so());
            if success {
                state.set_cr_field(0, 0b0010 | so);
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
                state.set_cr_field(0, so);
            }
            state.reservation = None;
            ExecuteVerdict::Continue
        }

        // Vector loads / stores
        PpuInstruction::Lvlx { vt, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let addr = base.wrapping_add(state.gpr[rb as usize]);
            let aligned = addr & !15u64;
            let val = match read_aligned_16(aligned, region_views, store_buf) {
                Ok(v) => v,
                Err(ea) => return ExecuteVerdict::MemFault(ea),
            };
            let shift = ((addr & 15) * 8) as u32;
            state.vr[vt as usize] = if shift == 0 { val } else { val << shift };
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
            state.vr[vt as usize] = if lo == 0 {
                0
            } else {
                val >> ((16 - lo) * 8) as u32
            };
            ExecuteVerdict::Continue
        }
        PpuInstruction::Stvx { vs, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]) & !15u64;
            // Split into two 8-byte halves so the reservation
            // clear-sweep in buffer_store sees both halves; matches
            // the dcbz block-store pattern. The two halves cover the
            // same 16-byte aligned line, so a covered reservation is
            // dropped on the first half regardless. Capacity is
            // pre-checked so a `BufferFull` mid-instruction cannot
            // leave half the line committed -- retry would duplicate
            // the first half's `SharedWriteIntent`.
            if !store_buf.has_capacity_for(2) {
                return ExecuteVerdict::BufferFull;
            }
            let bytes = state.vr[vs as usize].to_be_bytes();
            let hi = u64::from_be_bytes(bytes[0..8].try_into().unwrap());
            let lo = u64::from_be_bytes(bytes[8..16].try_into().unwrap());
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

        // Floating-point loads / stores
        PpuInstruction::Lfs { frt, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            match load_ze(region_views, store_buf, ea, 4) {
                Ok(bits) => {
                    state.fpr[frt as usize] = double_word(bits as u32);
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
            let bits = single_frs(state.fpr[frs as usize]);
            buffer_store(store_buf, state, ea, 4, bits as u64)
        }
        PpuInstruction::Stfd { frs, ra, imm } => {
            let ea = state.ea_d_form(ra, imm);
            buffer_store(store_buf, state, ea, 8, state.fpr[frs as usize])
        }
        PpuInstruction::Stfsu { frs, ra, imm } => {
            debug_assert_store_with_update("stfsu", ra);
            let ea = state.ea_d_form(ra, imm);
            let bits = single_frs(state.fpr[frs as usize]);
            let v = buffer_store(store_buf, state, ea, 4, bits as u64);
            if v == ExecuteVerdict::Continue {
                state.gpr[ra as usize] = ea;
            }
            v
        }
        PpuInstruction::Stfdu { frs, ra, imm } => {
            debug_assert_store_with_update("stfdu", ra);
            let ea = state.ea_d_form(ra, imm);
            let v = buffer_store(store_buf, state, ea, 8, state.fpr[frs as usize]);
            if v == ExecuteVerdict::Continue {
                state.gpr[ra as usize] = ea;
            }
            v
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

        // X-form FP indexed loads / stores. EA = (RA == 0 ? 0 : GPR[RA]) + GPR[RB].
        // The `u` (update) variants write EA back into GPR[RA] iff the
        // memory access succeeded, matching the D-form Stfsu/Stfdu policy.
        PpuInstruction::Lfsx { frt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            match load_ze(region_views, store_buf, ea, 4) {
                Ok(bits) => {
                    state.fpr[frt as usize] = double_word(bits as u32);
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Lfsux { frt, ra, rb } => {
            debug_assert_load_with_update("lfsux", ra, frt);
            let ea = state.ea_x_form(ra, rb);
            match load_ze(region_views, store_buf, ea, 4) {
                Ok(bits) => {
                    state.fpr[frt as usize] = double_word(bits as u32);
                    state.gpr[ra as usize] = ea;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Lfdx { frt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            match load_ze(region_views, store_buf, ea, 8) {
                Ok(bits) => {
                    state.fpr[frt as usize] = bits;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Lfdux { frt, ra, rb } => {
            debug_assert_load_with_update("lfdux", ra, frt);
            let ea = state.ea_x_form(ra, rb);
            match load_ze(region_views, store_buf, ea, 8) {
                Ok(bits) => {
                    state.fpr[frt as usize] = bits;
                    state.gpr[ra as usize] = ea;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Stfsx { frs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            let bits = single_frs(state.fpr[frs as usize]);
            buffer_store(store_buf, state, ea, 4, bits as u64)
        }
        PpuInstruction::Stfsux { frs, ra, rb } => {
            debug_assert_store_with_update("stfsux", ra);
            let ea = state.ea_x_form(ra, rb);
            let bits = single_frs(state.fpr[frs as usize]);
            let v = buffer_store(store_buf, state, ea, 4, bits as u64);
            if v == ExecuteVerdict::Continue {
                state.gpr[ra as usize] = ea;
            }
            v
        }
        PpuInstruction::Stfdx { frs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            buffer_store(store_buf, state, ea, 8, state.fpr[frs as usize])
        }
        PpuInstruction::Stfdux { frs, ra, rb } => {
            debug_assert_store_with_update("stfdux", ra);
            let ea = state.ea_x_form(ra, rb);
            let v = buffer_store(store_buf, state, ea, 8, state.fpr[frs as usize]);
            if v == ExecuteVerdict::Continue {
                state.gpr[ra as usize] = ea;
            }
            v
        }

        // Cache control
        PpuInstruction::Dcbz { ra, rb } => {
            let ea = state.ea_x_form(ra, rb) & !(DCBZ_BLOCK_BYTES as u64 - 1);
            debug_assert!(
                !(0xC000_0000..0xC010_0000).contains(&ea),
                "dcbz into RSX MMIO window at 0x{ea:x} likely indicates pointer corruption",
            );
            // 16 doubleword zero stores via the normal forwarding
            // path: load-after-dcbz in the same block sees zeros, and
            // the block-boundary flush emits the effect packets.
            // Capacity is pre-checked so a `BufferFull` mid-loop
            // cannot leave a partial block committed.
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

/// Execute `lvx` (Vx-form, xo=103): 16-byte aligned vector load.
/// EA is `((RA|0) + RB) & ~0xF`; the loaded line goes into VRT.
/// Lives here rather than in `vec` because the load path is the
/// same store-buffer-forward / region-view fallback shared with
/// `lvlx`/`lvrx`.
pub(crate) fn execute_lvx(
    state: &mut PpuState,
    vt: u8,
    ra: u8,
    rb: u8,
    region_views: &[(u64, &[u8])],
    store_buf: &StoreBuffer,
) -> ExecuteVerdict {
    let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
    let ea = base.wrapping_add(state.gpr[rb as usize]) & !15u64;
    let val = match read_aligned_16(ea, region_views, store_buf) {
        Ok(v) => v,
        Err(ea) => return ExecuteVerdict::MemFault(ea),
    };
    state.vr[vt as usize] = val;
    ExecuteVerdict::Continue
}

/// Resolve a 16-byte aligned vector-line read.
///
/// Fast path: a single buffered store fully covers the line, so
/// `forward` returns `Some` and the region is not touched.
///
/// Slow path: read the line from the region view and overlay any
/// partially-overlapping buffered stores byte-by-byte. This avoids
/// the otherwise-pessimistic flush+retry that scalar-store /
/// vector-load patterns on a shared cache line would force.
///
/// Returns `Err(aligned)` when no region view covers the line.
fn read_aligned_16(
    aligned: u64,
    region_views: &[(u64, &[u8])],
    store_buf: &StoreBuffer,
) -> Result<u128, u64> {
    if let Some(v) = store_buf.forward(aligned, 16) {
        return Ok(v);
    }
    let slice = match load_slice(region_views, aligned, 16) {
        Some(s) => s,
        None => return Err(aligned),
    };
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(slice);
    store_buf.overlay_range(aligned, &mut bytes);
    Ok(u128::from_be_bytes(bytes))
}

#[inline]
#[track_caller]
/// PPC `DOUBLE(WORD)` per Book I sec 4.6.2: convert a 32-bit
/// single-precision encoding to its 64-bit double encoding.
///
/// Matches Rust's `f32 as f64` for finite values, but preserves
/// NaN payloads bit-exactly (including the SNaN/QNaN distinction)
/// per Book I sec 4.6.2's bit-fill pseudocode for the
/// NaN/Inf/Zero branch. A naive `as f64` cast is allowed by Rust
/// to canonicalise NaNs and would silently quiet SNaNs
/// round-tripped through stfsx -> lfsx.
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

/// PPC `SINGLE(FRS)` per Book I sec 4.6.3: convert a 64-bit
/// double-precision encoding to its 32-bit single encoding.
///
/// Matches Rust's `f64 as f32` for finite values; preserves NaN
/// payloads bit-exactly (sign + high 23 fraction bits, exponent
/// reset to all-ones) per Book I sec 4.6.3's "No Denormalization
/// Required" branch on NaN inputs.
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

fn debug_assert_load_with_update(insn: &str, ra: u8, rt: u8) {
    // PPC Book I 3.3.2: invalid form when RA=0 (no base) or RA=RT
    // (the EA-write would clobber the loaded value). Real games
    // never encode this; assemblers reject it. Surfaces in tests if
    // a decoder bug or self-modifying code produces it.
    debug_assert!(ra != 0 && ra != rt, "{insn} invalid form: RA={ra}, RT={rt}");
}

#[inline]
#[track_caller]
fn debug_assert_store_with_update(insn: &str, ra: u8) {
    // PPC Book I 3.3.3: store-with-update with RA=0 has no base
    // register to update; assemblers reject the encoding.
    debug_assert!(ra != 0, "{insn} invalid form: RA=0");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::execute;
    use crate::exec::test_support::{exec_no_mem, exec_with_mem, uid};
    use cellgov_event::UnitId;
    use cellgov_sync::ReservedLine;

    #[test]
    fn ldu_writes_ea_back_to_ra() {
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
        assert_eq!(s.gpr[4], 0x1018);
    }

    #[test]
    fn lwz_loads_from_memory() {
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
        // 0xFF80 == -128 as i16; lha sign-extends to the full GPR width.
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
    fn lwa_sign_extends_word_into_64_bits() {
        // 0xFFFF_FFFE = -2 as i32; lwa must sign-extend to the full
        // 64-bit GPR. Reading this as lwz (zero-extend) would give
        // 0x0000_0000_FFFF_FFFE instead.
        let mut mem = vec![0u8; 0x2000];
        mem[0x1004..0x1008].copy_from_slice(&0xFFFF_FFFEu32.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        let mut effects = Vec::new();
        let result = exec_with_mem(
            &PpuInstruction::Lwa {
                rt: 3,
                ra: 1,
                imm: 4,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(result, ExecuteVerdict::Continue);
        assert_eq!(s.gpr[3], 0xFFFF_FFFF_FFFF_FFFE);
        assert_eq!(s.gpr[3] as i64, -2);
    }

    #[test]
    fn lwa_sign_extends_through_store_buffer_forward() {
        // stw 0xFFFF_FFFE then lwa from the same EA: the load goes
        // through StoreBuffer::forward, which packs the stored bytes
        // right-aligned into a u128. lwa must sign-extend from the
        // top of the 32-bit access (bit 31), not from u64 bit 63 --
        // sub-8-byte store-buffer forwards leave high u64 bits zero,
        // so a `val as i64 as u64` sign-extend from bit 63 always
        // produces a positive value regardless of the stored sign.
        // Pins the size-aware sign extension.
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        s.gpr[5] = 0xFFFF_FFFE;
        let mut effects = Vec::new();
        let mut store_buf = StoreBuffer::new();
        let region_views: [(u64, &[u8]); 1] = [(0, &[0u8; 0x2000])];
        let v_stw = execute(
            &PpuInstruction::Stw {
                rs: 5,
                ra: 1,
                imm: 0,
            },
            &mut s,
            UnitId::new(0),
            &region_views,
            &mut effects,
            &mut store_buf,
        );
        assert_eq!(v_stw, ExecuteVerdict::Continue);
        let v_lwa = execute(
            &PpuInstruction::Lwa {
                rt: 3,
                ra: 1,
                imm: 0,
            },
            &mut s,
            UnitId::new(0),
            &region_views,
            &mut effects,
            &mut store_buf,
        );
        assert_eq!(v_lwa, ExecuteVerdict::Continue);
        assert_eq!(s.gpr[3], 0xFFFF_FFFF_FFFF_FFFE);
        assert_eq!(s.gpr[3] as i64, -2);
    }

    #[test]
    fn lha_sign_extends_through_store_buffer_forward() {
        // sth 0xFF80 then lha: same forwarding bug at halfword width.
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        s.gpr[5] = 0xFF80;
        let mut effects = Vec::new();
        let mut store_buf = StoreBuffer::new();
        let region_views: [(u64, &[u8]); 1] = [(0, &[0u8; 0x2000])];
        let v_sth = execute(
            &PpuInstruction::Sth {
                rs: 5,
                ra: 1,
                imm: 0,
            },
            &mut s,
            UnitId::new(0),
            &region_views,
            &mut effects,
            &mut store_buf,
        );
        assert_eq!(v_sth, ExecuteVerdict::Continue);
        let v_lha = execute(
            &PpuInstruction::Lha {
                rt: 3,
                ra: 1,
                imm: 0,
            },
            &mut s,
            UnitId::new(0),
            &region_views,
            &mut effects,
            &mut store_buf,
        );
        assert_eq!(v_lha, ExecuteVerdict::Continue);
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
    fn lhzu_loads_halfword_and_updates_base() {
        let mut mem = vec![0u8; 0x2000];
        mem[0x1010..0x1012].copy_from_slice(&0xBEEFu16.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[4] = 0x1000;
        let mut effects = Vec::new();
        let result = exec_with_mem(
            &PpuInstruction::Lhzu {
                rt: 3,
                ra: 4,
                imm: 0x10,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(result, ExecuteVerdict::Continue);
        assert_eq!(s.gpr[3], 0xBEEF);
        assert_eq!(s.gpr[4], 0x1010);
    }

    #[test]
    fn stdux_stores_doubleword_and_updates_base() {
        let mem = vec![0u8; 0x2000];
        let mut s = PpuState::new();
        s.gpr[3] = 0xDEAD_BEEF_CAFE_F00D;
        s.gpr[4] = 0x1000;
        s.gpr[5] = 0x40;
        let mut effects = Vec::new();
        let result = exec_with_mem(
            &PpuInstruction::Stdux {
                rs: 3,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(result, ExecuteVerdict::Continue);
        assert_eq!(s.gpr[4], 0x1040);
        assert!(!effects.is_empty());
    }

    #[test]
    fn stbu_updates_ra_with_effective_address() {
        let mem = vec![0u8; 0x100];
        let mut s = PpuState::new();
        s.gpr[1] = 0x20;
        s.gpr[6] = 0xAB;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stbu {
                rs: 6,
                ra: 1,
                imm: -4,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        // EA = 0x20 + (-4) = 0x1C; RA receives EA after the store.
        assert_eq!(s.gpr[1], 0x1C);
        let found = effects.iter().any(|e| match e {
            Effect::SharedWriteIntent { range, .. } => range.start().raw() == 0x1C,
            _ => false,
        });
        assert!(found, "stbu should emit a byte store at EA");
    }

    #[test]
    fn sthu_updates_ra_with_effective_address() {
        let mem = vec![0u8; 0x100];
        let mut s = PpuState::new();
        s.gpr[1] = 0x40;
        s.gpr[5] = 0xBEEF;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Sthu {
                rs: 5,
                ra: 1,
                imm: -8,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[1], 0x38);
        let found = effects.iter().any(|e| match e {
            Effect::SharedWriteIntent { range, .. } => range.start().raw() == 0x38,
            _ => false,
        });
        assert!(found, "sthu should emit a halfword store at EA");
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
    fn stdcx_with_matching_reservation_emits_conditional_store() {
        let mut s = PpuState::new();
        // Pre-seed the reservation the way a prior ldarx at this line would.
        s.reservation = Some(ReservedLine::containing(0x1008));
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
        assert_eq!(s.cr_field(0), 0b0010);
        assert!(s.reservation.is_none());
        // stdcx must emit ConditionalStore, never a SharedWriteIntent.
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::ConditionalStore { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x1008);
                assert_eq!(range.length(), 8);
                assert_eq!(bytes.bytes(), &0xDEAD_BEEF_CAFE_BABEu64.to_be_bytes());
            }
            other => panic!("expected ConditionalStore, got {other:?}"),
        }
    }

    #[test]
    fn stdcx_without_reservation_fails_silently() {
        let mut s = PpuState::new();
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x8;
        s.gpr[5] = 0xDEAD_BEEF_CAFE_BABE;
        let mut effects = Vec::new();
        exec_with_mem(
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
        assert_eq!(s.cr_field(0), 0b0000);
        assert!(effects.is_empty());
    }

    #[test]
    fn stwcx_with_reservation_on_different_line_fails() {
        // 128-byte reservation granule: 0x1000 and 0x1080 sit on different lines.
        let mut s = PpuState::new();
        s.reservation = Some(ReservedLine::containing(0x1000));
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x80;
        s.gpr[5] = 0xDEAD_BEEF;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stwcx {
                rs: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            0,
            &[0u8; 0x2000],
            &mut effects,
        );
        assert_eq!(s.cr_field(0), 0b0000);
        assert!(effects.is_empty());
        // PowerPC ABI: stwcx retires the reservation even on failure.
        assert!(s.reservation.is_none());
    }

    #[test]
    fn same_unit_store_to_reserved_line_clears_local_reservation() {
        // Cross-unit contract: any plain store overlapping the reserved
        // 128-byte line must drop the local reservation so a later stwcx
        // on that same line fails.
        let mut mem = vec![0u8; 0x2000];
        mem[0x1000..0x1004].copy_from_slice(&0xdeadbeefu32.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x0;
        let mut effects = Vec::new();

        exec_with_mem(
            &PpuInstruction::Lwarx {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.reservation.map(|l| l.addr()), Some(0x1000));

        s.gpr[6] = 0x1040;
        s.gpr[7] = 0xAAAA_BBBBu64;
        let mut effects2 = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stw {
                rs: 7,
                ra: 6,
                imm: 0,
            },
            &mut s,
            0,
            &mem,
            &mut effects2,
        );
        assert!(
            s.reservation.is_none(),
            "same-unit store to reserved line must drop the local reservation"
        );

        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x0;
        s.gpr[5] = 0x5555_6666u64;
        let mut effects3 = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stwcx {
                rs: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            0,
            &[0u8; 0x2000],
            &mut effects3,
        );
        assert_eq!(
            s.cr_field(0),
            0b0000,
            "stwcx must fail after self-invalidation"
        );
        assert!(effects3.is_empty());
    }

    #[test]
    fn lwarx_sets_local_reservation_and_emits_acquire() {
        let mut mem = vec![0u8; 0x2000];
        mem[0x1040..0x1044].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x40;
        let mut effects = Vec::new();
        let result = exec_with_mem(
            &PpuInstruction::Lwarx {
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
        assert_eq!(s.gpr[5], 0xDEAD_BEEF);
        // Reservation tracks the enclosing 128-byte line, not the raw EA.
        assert_eq!(
            s.reservation.map(|l| l.addr()),
            Some(0x1000),
            "local reservation must be set to the enclosing line"
        );
        let acquires: Vec<_> = effects
            .iter()
            .filter_map(|e| match e {
                Effect::ReservationAcquire { line_addr, source } => Some((*line_addr, *source)),
                _ => None,
            })
            .collect();
        assert_eq!(acquires, vec![(0x1000, UnitId::new(0))]);
    }

    #[test]
    fn ldarx_sets_local_reservation_and_emits_acquire() {
        let mem = vec![0u8; 0x2000];
        let mut s = PpuState::new();
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x8;
        let mut effects = Vec::new();
        exec_with_mem(
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
        assert_eq!(s.reservation.map(|l| l.addr()), Some(0x1000));
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::ReservationAcquire {
                line_addr: 0x1000,
                ..
            }
        )));
    }

    #[test]
    fn stwcx_on_matching_line_retires_local_reservation() {
        let mut s = PpuState::new();
        s.reservation = Some(ReservedLine::containing(0x1000));
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x0;
        s.gpr[5] = 0xCAFE_BABE;
        let mut effects = Vec::new();
        let result = exec_with_mem(
            &PpuInstruction::Stwcx {
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
        assert_eq!(s.cr_field(0), 0b0010);
        assert!(
            s.reservation.is_none(),
            "stwcx must retire the local reservation on success"
        );
    }

    #[test]
    fn stdcx_on_matching_line_retires_local_reservation() {
        let mut s = PpuState::new();
        s.reservation = Some(ReservedLine::containing(0x1000));
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x0;
        s.gpr[5] = 0xDEAD_BEEF_CAFE_BABE;
        let mut effects = Vec::new();
        exec_with_mem(
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
        assert!(s.reservation.is_none());
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
        // stvx forces EA to 16-byte alignment: 0x1000+0x1F -> 0x1010, then
        // commits as two 8-byte halves so buffer_store's reservation
        // clear-sweep covers both.
        assert_eq!(effects.len(), 2);
        match &effects[0] {
            Effect::SharedWriteIntent { range, .. } => {
                assert_eq!(range.start().raw(), 0x1010);
                assert_eq!(range.length(), 8);
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
        match &effects[1] {
            Effect::SharedWriteIntent { range, .. } => {
                assert_eq!(range.start().raw(), 0x1018);
                assert_eq!(range.length(), 8);
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn stvx_clears_overlapping_reservation() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        s.gpr[8] = 0;
        s.vr[0] = 0u128;
        s.reservation = Some(ReservedLine::containing(0x1000));
        let mut effects = Vec::new();
        exec_with_mem(
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
        assert!(
            s.reservation.is_none(),
            "stvx covering the reserved line must drop the reservation"
        );
    }

    #[test]
    fn lvlx_aligned_address_matches_lvx() {
        // 16-aligned EA degenerates lvlx to lvx: zero-bit shift.
        let mut mem = vec![0u8; 0x2000];
        let pattern = [
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE,
            0xFF, 0x00,
        ];
        mem[0x1000..0x1010].copy_from_slice(&pattern);
        let mut s = PpuState::new();
        s.gpr[4] = 0x1000;
        s.gpr[5] = 0;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lvlx {
                vt: 7,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.vr[7], u128::from_be_bytes(pattern));
    }

    #[test]
    fn lvlx_unaligned_shifts_high_bytes_up() {
        // lvlx: result = (aligned_block << (EA & 15) * 8), low bytes zeroed.
        let mut mem = vec![0u8; 0x2000];
        let pattern = [
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE,
            0xFF, 0x10,
        ];
        mem[0x1000..0x1010].copy_from_slice(&pattern);
        let mut s = PpuState::new();
        s.gpr[4] = 0x1003;
        s.gpr[5] = 0;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lvlx {
                vt: 7,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        let expected = u128::from_be_bytes(pattern) << 24;
        assert_eq!(s.vr[7], expected);
    }

    #[test]
    fn lvrx_unaligned_shifts_low_bytes_down() {
        // lvrx: result = (aligned_block >> (16 - (EA & 15)) * 8), high bytes zeroed.
        let mut mem = vec![0u8; 0x2000];
        let pattern = [
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE,
            0xFF, 0x10,
        ];
        mem[0x1000..0x1010].copy_from_slice(&pattern);
        let mut s = PpuState::new();
        s.gpr[4] = 0x1003;
        s.gpr[5] = 0;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lvrx {
                vt: 7,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        let expected = u128::from_be_bytes(pattern) >> 104;
        assert_eq!(s.vr[7], expected);
    }

    #[test]
    fn lvrx_aligned_ea_zero_bytes() {
        // 16-aligned EA: (16 - 0)*8 == 128-bit shift, so lvrx result is zero.
        let mut mem = vec![0u8; 0x2000];
        mem[0x1000..0x1010].copy_from_slice(&[0xFF; 16]);
        let mut s = PpuState::new();
        s.gpr[4] = 0x1000;
        s.gpr[5] = 0;
        s.vr[7] = u128::MAX;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lvrx {
                vt: 7,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.vr[7], 0);
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
        assert_eq!(s.gpr[8], 0x2008);
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
        // stfiwx writes the low 32 bits of the FPR bit pattern verbatim;
        // no single-precision round-convert (unlike stfs).
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
                assert_eq!(bytes.bytes(), &0x1234_5678u32.to_be_bytes());
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn lfsx_loads_single_and_round_converts_to_double() {
        // 1.5f as float bits is 0x3FC00000; verify the FPR holds the
        // double bit pattern of 1.5 (0x3FF8000000000000).
        let mut mem = vec![0u8; 0x100];
        mem[0x40..0x44].copy_from_slice(&0x3FC0_0000u32.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[4] = 0x40;
        s.gpr[5] = 0;
        let mut effects = Vec::new();
        let out = exec_with_mem(
            &PpuInstruction::Lfsx {
                frt: 7,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(out, ExecuteVerdict::Continue);
        assert_eq!(s.fpr[7], 0x3FF8_0000_0000_0000);
    }

    #[test]
    fn lfsux_writes_back_ea_to_ra() {
        let mut mem = vec![0u8; 0x100];
        mem[0x44..0x48].copy_from_slice(&0x4040_0000u32.to_be_bytes()); // 3.0f
        let mut s = PpuState::new();
        s.gpr[4] = 0x40;
        s.gpr[5] = 4;
        let mut effects = Vec::new();
        let out = exec_with_mem(
            &PpuInstruction::Lfsux {
                frt: 8,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(out, ExecuteVerdict::Continue);
        assert_eq!(s.gpr[4], 0x44);
        // 3.0 as double: 0x4008000000000000
        assert_eq!(s.fpr[8], 0x4008_0000_0000_0000);
    }

    #[test]
    fn lfdx_loads_64_bit_double() {
        let mut mem = vec![0u8; 0x100];
        let bits = 0x4080_1122_3344_5566u64;
        mem[0x10..0x18].copy_from_slice(&bits.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[2] = 0x10;
        s.gpr[3] = 0;
        let mut effects = Vec::new();
        let out = exec_with_mem(
            &PpuInstruction::Lfdx {
                frt: 9,
                ra: 2,
                rb: 3,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(out, ExecuteVerdict::Continue);
        assert_eq!(s.fpr[9], bits);
    }

    #[test]
    fn lfdux_writes_back_ea_to_ra() {
        let mut mem = vec![0u8; 0x100];
        let bits = 0x4090_AAAA_BBBB_CCCCu64;
        mem[0x20..0x28].copy_from_slice(&bits.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[2] = 0x10;
        s.gpr[3] = 0x10;
        let mut effects = Vec::new();
        let out = exec_with_mem(
            &PpuInstruction::Lfdux {
                frt: 10,
                ra: 2,
                rb: 3,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(out, ExecuteVerdict::Continue);
        assert_eq!(s.gpr[2], 0x20);
        assert_eq!(s.fpr[10], bits);
    }

    #[test]
    fn stfsx_stores_round_converted_single() {
        let mut s = PpuState::new();
        s.gpr[4] = 0x100;
        s.gpr[5] = 0x4;
        // 1.5 as double; round-convert to single bit pattern is 0x3FC00000.
        s.fpr[6] = 0x3FF8_0000_0000_0000;
        let mut effects = Vec::new();
        let out = exec_with_mem(
            &PpuInstruction::Stfsx {
                frs: 6,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        assert_eq!(out, ExecuteVerdict::Continue);
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x104);
                assert_eq!(bytes.bytes(), &0x3FC0_0000u32.to_be_bytes());
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn stfsux_writes_back_ea_only_on_success() {
        let mut s = PpuState::new();
        s.gpr[4] = 0x40;
        s.gpr[5] = 0x4;
        s.fpr[3] = 0x4040_0000_0000_0000; // 32.0 as double
        let mut effects = Vec::new();
        let out = exec_with_mem(
            &PpuInstruction::Stfsux {
                frs: 3,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        assert_eq!(out, ExecuteVerdict::Continue);
        assert_eq!(s.gpr[4], 0x44);
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn stfdx_stores_64_bit_double_verbatim() {
        let mut s = PpuState::new();
        s.gpr[4] = 0x80;
        s.gpr[5] = 0x10;
        let bits = 0xC020_FFFF_0000_1111u64;
        s.fpr[2] = bits;
        let mut effects = Vec::new();
        let out = exec_with_mem(
            &PpuInstruction::Stfdx {
                frs: 2,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        assert_eq!(out, ExecuteVerdict::Continue);
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x90);
                assert_eq!(bytes.bytes(), &bits.to_be_bytes());
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn lfsx_preserves_nan_payload_bit_for_bit() {
        // SNaN single (high frac bit clear, payload non-zero):
        // 0x7F801234. Spec says lfsx delivers WORD0:1 + WORD2:31||0^29
        // into FRT, leaving the SNaN/QNaN distinction untouched.
        let mut mem = vec![0u8; 0x100];
        mem[0x10..0x14].copy_from_slice(&0x7F80_1234u32.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[3] = 0x10;
        s.gpr[4] = 0;
        let mut effects = Vec::new();
        let out = exec_with_mem(
            &PpuInstruction::Lfsx {
                frt: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(out, ExecuteVerdict::Continue);
        // Expected: sign=0, exp=0x7FF, frac52 = 0x001234 << 29.
        let expected = (0x7FFu64 << 52) | (0x001234u64 << 29);
        assert_eq!(s.fpr[5], expected);
    }

    #[test]
    fn stfsx_preserves_nan_payload_bit_for_bit() {
        // FRS = double-encoded NaN with sign=1, exp=0x7FF,
        // frac52 = 0xABCDE_DEADBEEF (low 29 bits will be discarded
        // by the spec's WORD2:31 <- FRS5:34 selection). Expect WORD
        // = sign=1, exp=0xFF, frac23 = top 23 bits of frac52.
        let mut s = PpuState::new();
        s.gpr[4] = 0x80;
        s.gpr[5] = 0;
        let frac52: u64 = 0x000A_BCDE_DEAD_BEEF;
        let nan_d = (1u64 << 63) | (0x7FFu64 << 52) | frac52;
        s.fpr[6] = nan_d;
        let mut effects = Vec::new();
        let out = exec_with_mem(
            &PpuInstruction::Stfsx {
                frs: 6,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        assert_eq!(out, ExecuteVerdict::Continue);
        let frac23 = ((frac52 >> 29) & 0x007F_FFFF) as u32;
        let expected = (1u32 << 31) | (0xFFu32 << 23) | frac23;
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x80);
                assert_eq!(bytes.bytes(), &expected.to_be_bytes());
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn stfsx_then_lfsx_round_trips_nan_payload() {
        // Round-trip pin: a NaN whose 23 high fraction bits are
        // distinct survives stfsx -> lfsx with the same single bit
        // pattern, after re-expansion in lfsx into double form.
        let frac23: u32 = 0x004A_5A5A;
        let single_nan = (1u32 << 31) | (0xFFu32 << 23) | frac23;
        // Set up FPR with the canonical lfsx-of-this-single result.
        let canonical_fpr = (1u64 << 63) | (0x7FFu64 << 52) | ((frac23 as u64) << 29);

        // stfsx round.
        let mut s = PpuState::new();
        s.gpr[4] = 0x80;
        s.gpr[5] = 0;
        s.fpr[7] = canonical_fpr;
        let mut effects = Vec::new();
        let out = exec_with_mem(
            &PpuInstruction::Stfsx {
                frs: 7,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        assert_eq!(out, ExecuteVerdict::Continue);
        let stored = match &effects[0] {
            Effect::SharedWriteIntent { bytes, .. } => {
                u32::from_be_bytes(bytes.bytes().try_into().unwrap())
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        };
        assert_eq!(stored, single_nan, "stfsx must preserve NaN bit pattern");

        // lfsx round.
        let mut mem = vec![0u8; 0x100];
        mem[0x40..0x44].copy_from_slice(&single_nan.to_be_bytes());
        let mut s2 = PpuState::new();
        s2.gpr[3] = 0x40;
        s2.gpr[4] = 0;
        let mut effects2 = Vec::new();
        let out = exec_with_mem(
            &PpuInstruction::Lfsx {
                frt: 8,
                ra: 3,
                rb: 4,
            },
            &mut s2,
            0,
            &mem,
            &mut effects2,
        );
        assert_eq!(out, ExecuteVerdict::Continue);
        assert_eq!(
            s2.fpr[8], canonical_fpr,
            "lfsx-of-NaN must rebuild the spec FPR pattern bit-for-bit"
        );
    }

    #[test]
    fn lfsux_load_fault_does_not_write_ra() {
        // EA out of mapped region: load_ze returns Err(ea), the
        // handler emits MemFault, and RA must stay at its prior
        // value. A naive implementation that writes RA before
        // checking the load result would break the on-success-only
        // discipline.
        let mem = vec![0u8; 0x100];
        let mut s = PpuState::new();
        s.gpr[4] = 0x1000_0000; // far outside the 0x100-byte region
        s.gpr[5] = 0;
        let original_ra = s.gpr[4];
        let mut effects = Vec::new();
        let out = exec_with_mem(
            &PpuInstruction::Lfsux {
                frt: 9,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert!(matches!(out, ExecuteVerdict::MemFault(_)));
        assert_eq!(s.gpr[4], original_ra);
    }

    #[test]
    fn stfsux_buffer_full_does_not_write_ra() {
        // Pre-fill the store buffer to capacity, then dispatch an
        // stfsux. buffer_store should return BufferFull; RA must
        // remain at its prior value so the retry-after-flush sees
        // the same architectural state.
        use crate::store_buffer::StoreBuffer;
        let mut store_buf = StoreBuffer::new();
        for i in 0..64 {
            assert!(store_buf.insert(0x1000 + i * 4, 4, i as u128));
        }
        assert!(store_buf.is_full());
        let mut s = PpuState::new();
        s.gpr[4] = 0x80;
        s.gpr[5] = 0x10;
        s.fpr[3] = 0x4040_0000_0000_0000;
        let original_ra = s.gpr[4];
        let mem = [0u8; 0x200];
        let views: [(u64, &[u8]); 1] = [(0, &mem)];
        let mut effects = Vec::new();
        let out = crate::exec::execute(
            &PpuInstruction::Stfsux {
                frs: 3,
                ra: 4,
                rb: 5,
            },
            &mut s,
            uid(),
            &views,
            &mut effects,
            &mut store_buf,
        );
        assert_eq!(out, ExecuteVerdict::BufferFull);
        assert_eq!(s.gpr[4], original_ra);
    }

    #[test]
    fn stfdux_buffer_full_does_not_write_ra() {
        use crate::store_buffer::StoreBuffer;
        let mut store_buf = StoreBuffer::new();
        for i in 0..64 {
            assert!(store_buf.insert(0x2000 + i * 4, 4, i as u128));
        }
        let mut s = PpuState::new();
        s.gpr[4] = 0x60;
        s.gpr[5] = 0x8;
        s.fpr[2] = 0xDEAD_BEEF_CAFE_BABE;
        let original_ra = s.gpr[4];
        let mem = [0u8; 0x200];
        let views: [(u64, &[u8]); 1] = [(0, &mem)];
        let mut effects = Vec::new();
        let out = crate::exec::execute(
            &PpuInstruction::Stfdux {
                frs: 2,
                ra: 4,
                rb: 5,
            },
            &mut s,
            uid(),
            &views,
            &mut effects,
            &mut store_buf,
        );
        assert_eq!(out, ExecuteVerdict::BufferFull);
        assert_eq!(s.gpr[4], original_ra);
    }

    #[test]
    fn lfdux_load_fault_does_not_write_ra() {
        let mem = vec![0u8; 0x100];
        let mut s = PpuState::new();
        s.gpr[2] = 0x2000_0000;
        s.gpr[3] = 0;
        let original_ra = s.gpr[2];
        let mut effects = Vec::new();
        let out = exec_with_mem(
            &PpuInstruction::Lfdux {
                frt: 10,
                ra: 2,
                rb: 3,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert!(matches!(out, ExecuteVerdict::MemFault(_)));
        assert_eq!(s.gpr[2], original_ra);
    }

    #[test]
    fn stfdux_writes_back_ea_to_ra() {
        let mut s = PpuState::new();
        s.gpr[4] = 0x60;
        s.gpr[5] = 0x8;
        s.fpr[2] = 0xDEAD_BEEF_CAFE_BABE;
        let mut effects = Vec::new();
        let out = exec_with_mem(
            &PpuInstruction::Stfdux {
                frs: 2,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        assert_eq!(out, ExecuteVerdict::Continue);
        assert_eq!(s.gpr[4], 0x68);
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn dcbz_zeroes_128_byte_aligned_block() {
        // Buffer spans three cache lines with sentinel bytes before and
        // after the target block so we can confirm dcbz does not spill.
        let base = 0x1000u64;
        let mut mem = vec![0xAAu8; 384];
        let mut s = PpuState::new();
        // ea = 0x1000 + 100 = 0x1064; aligned block starts at 0x1000.
        s.gpr[3] = 100;
        s.gpr[4] = 0x1000;
        let mut effects = Vec::new();
        let v = exec_with_mem(
            &PpuInstruction::Dcbz { ra: 3, rb: 4 },
            &mut s,
            base,
            &mem,
            &mut effects,
        );
        assert_eq!(v, ExecuteVerdict::Continue);
        // Reconstruct what committed memory would look like after the 16
        // SharedWriteIntent effects land.
        for eff in &effects {
            if let Effect::SharedWriteIntent { range, bytes, .. } = eff {
                let start = (range.start().raw() - base) as usize;
                let end = start + range.length() as usize;
                mem[start..end].copy_from_slice(bytes.bytes());
            }
        }
        // Bytes outside the 128-byte aligned block are untouched.
        for (i, b) in mem.iter().enumerate().take(384) {
            let in_block = i < 128;
            let expected = if in_block { 0 } else { 0xAA };
            assert_eq!(*b, expected, "mem[{i}] (in_block={in_block})");
        }
        assert_eq!(effects.len(), 16, "16 doubleword zero effects");
    }

    #[test]
    fn dcbz_ea_is_aligned_down_to_block_boundary() {
        let base = 0x2000u64;
        let mem = vec![0xFFu8; 256];
        let mut s = PpuState::new();
        // EA = 0x2000 + 0x7F = 0x207F. Aligned EA = 0x2000.
        s.gpr[3] = 0x7F;
        s.gpr[4] = 0x2000;
        let mut effects = Vec::new();
        let v = exec_with_mem(
            &PpuInstruction::Dcbz { ra: 3, rb: 4 },
            &mut s,
            base,
            &mem,
            &mut effects,
        );
        assert_eq!(v, ExecuteVerdict::Continue);
        // All effect ranges must start in [0x2000, 0x2080).
        for eff in &effects {
            if let Effect::SharedWriteIntent { range, .. } = eff {
                let addr = range.start().raw();
                assert!(
                    (0x2000..0x2080).contains(&addr),
                    "effect addr 0x{addr:x} outside aligned block [0x2000, 0x2080)",
                );
            }
        }
    }

    #[test]
    fn stfsu_buffer_full_does_not_update_ra() {
        // Mirrors the integer Stwu contract: when buffer_store returns
        // BufferFull the update of RA must be skipped, so the caller can
        // retry after flushing without double-advancing the base pointer.
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        s.fpr[5] = (1.5f32 as f64).to_bits();
        let mut store_buf = StoreBuffer::new();
        // Fill the buffer to capacity. CAPACITY is private; saturate by
        // inserting until insert reports `is_full`.
        while !store_buf.is_full() {
            assert!(store_buf.insert(0, 1, 0));
        }
        let mut effects = Vec::new();
        let v = execute(
            &PpuInstruction::Stfsu {
                frs: 5,
                ra: 1,
                imm: 0x40,
            },
            &mut s,
            uid(),
            &[],
            &mut effects,
            &mut store_buf,
        );
        assert_eq!(v, ExecuteVerdict::BufferFull);
        assert_eq!(
            s.gpr[1], 0x1000,
            "RA must be unchanged when buffer_store returns BufferFull"
        );
    }

    #[test]
    fn stfdu_buffer_full_does_not_update_ra() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x2000;
        s.fpr[6] = 0xDEAD_BEEF_CAFE_F00Du64;
        let mut store_buf = StoreBuffer::new();
        while !store_buf.is_full() {
            assert!(store_buf.insert(0, 1, 0));
        }
        let mut effects = Vec::new();
        let v = execute(
            &PpuInstruction::Stfdu {
                frs: 6,
                ra: 1,
                imm: 0x80,
            },
            &mut s,
            uid(),
            &[],
            &mut effects,
            &mut store_buf,
        );
        assert_eq!(v, ExecuteVerdict::BufferFull);
        assert_eq!(
            s.gpr[1], 0x2000,
            "RA must be unchanged when buffer_store returns BufferFull"
        );
    }

    #[test]
    fn stfd_clears_overlapping_reservation() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        s.fpr[5] = 0xDEAD_BEEF_CAFE_F00Du64;
        s.reservation = Some(ReservedLine::containing(0x1000));
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stfd {
                frs: 5,
                ra: 1,
                imm: 0,
            },
            &mut s,
            0,
            &[0u8; 0x2000],
            &mut effects,
        );
        assert!(
            s.reservation.is_none(),
            "stfd covering the reserved line must drop the reservation"
        );
    }

    #[test]
    fn stwcx_success_propagates_xer_so_into_cr0() {
        // Book II 3.3.2: stwcx CR0 = 0b00 || n || XER[SO]. Earlier
        // code zeroed the SO bit unconditionally.
        let mut s = PpuState::new();
        s.reservation = Some(ReservedLine::containing(0x1000));
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0;
        s.gpr[5] = 0xDEAD_BEEF;
        s.set_xer_ov(true); // sets SO sticky
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stwcx {
                rs: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            0,
            &[0u8; 0x2000],
            &mut effects,
        );
        // success (0b0010) | SO (0b0001) = 0b0011.
        assert_eq!(s.cr_field(0), 0b0011);
    }

    #[test]
    fn stwcx_failure_propagates_xer_so_into_cr0() {
        let mut s = PpuState::new();
        // No reservation: stwcx fails.
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0;
        s.gpr[5] = 0;
        s.set_xer_ov(true);
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stwcx {
                rs: 5,
                ra: 3,
                rb: 4,
            },
            &mut s,
            0,
            &[0u8; 0x2000],
            &mut effects,
        );
        // failure (0b0000) | SO (0b0001) = 0b0001.
        assert_eq!(s.cr_field(0), 0b0001);
    }

    #[test]
    fn stdcx_success_propagates_xer_so_into_cr0() {
        let mut s = PpuState::new();
        s.reservation = Some(ReservedLine::containing(0x1008));
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x8;
        s.gpr[5] = 0xDEAD_BEEF_CAFE_BABE;
        s.set_xer_ov(true);
        let mut effects = Vec::new();
        exec_with_mem(
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
        assert_eq!(s.cr_field(0), 0b0011);
    }

    #[test]
    fn stvx_pre_checks_capacity_for_both_halves() {
        // Pre-fill the buffer to 63 entries so only one slot remains
        // -- not enough for stvx's two halves. Without the
        // pre-check, the first half would commit and the retry
        // would duplicate it.
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        s.gpr[2] = 0;
        s.vr[3] = 0xAABB_CCDD_EEFF_0011_2233_4455_6677_8899u128;
        let mut effects = Vec::new();
        let mut store_buf = StoreBuffer::new();
        for i in 0..63 {
            assert!(store_buf.insert((i as u64) * 8, 8, 0));
        }
        let v = execute(
            &PpuInstruction::Stvx {
                vs: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            UnitId::new(0),
            &[(0, &[0u8; 0x2000])],
            &mut effects,
            &mut store_buf,
        );
        assert_eq!(v, ExecuteVerdict::BufferFull);
        assert_eq!(
            store_buf.len(),
            63,
            "no partial commit when capacity is insufficient"
        );
    }

    #[test]
    fn dcbz_pre_checks_capacity_for_full_block() {
        // Buffer with 50 entries leaves 14 slots -- not enough for
        // dcbz's 16 doubleword stores.
        let mut s = PpuState::new();
        s.gpr[1] = 0x2000;
        s.gpr[2] = 0;
        let mut effects = Vec::new();
        let mut store_buf = StoreBuffer::new();
        for i in 0..50 {
            assert!(store_buf.insert((i as u64) * 8, 8, 0));
        }
        let v = execute(
            &PpuInstruction::Dcbz { ra: 1, rb: 2 },
            &mut s,
            UnitId::new(0),
            &[(0, &[0u8; 0x4000])],
            &mut effects,
            &mut store_buf,
        );
        assert_eq!(v, ExecuteVerdict::BufferFull);
        assert_eq!(
            store_buf.len(),
            50,
            "dcbz must not stage any stores when capacity is insufficient"
        );
    }

    #[test]
    fn lvlx_partial_overlap_merges_buffered_bytes_with_region() {
        // Pre-stage a 4-byte store at offset +4 within the line.
        // forward(aligned, 16) returns None (no full match). The
        // load reads the 16-byte line from regions and overlays the
        // 4 buffered bytes byte-by-byte instead of yielding.
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        s.gpr[2] = 0;
        let mut effects = Vec::new();
        let mut store_buf = StoreBuffer::new();
        assert!(store_buf.insert(0x1004, 4, 0xDEAD_BEEFu128));
        let mut mem = vec![0u8; 0x2000];
        for i in 0..16 {
            mem[0x1000 + i] = 0x10 + i as u8;
        }
        let v = execute(
            &PpuInstruction::Lvlx {
                vt: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            UnitId::new(0),
            &[(0, &mem)],
            &mut effects,
            &mut store_buf,
        );
        assert_eq!(v, ExecuteVerdict::Continue);
        // Lvlx with aligned EA shifts by 0, so the result is the
        // raw 16 bytes. Bytes 4..8 should be patched to DEADBEEF.
        let expected = u128::from_be_bytes([
            0x10, 0x11, 0x12, 0x13, // unchanged
            0xDE, 0xAD, 0xBE, 0xEF, // patched
            0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F,
        ]);
        assert_eq!(s.vr[3], expected);
    }

    #[test]
    fn stdcx_failure_propagates_xer_so_into_cr0() {
        // Symmetric to the success test; covers the failure code
        // path which writes a different CR0 value.
        let mut s = PpuState::new();
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x8;
        s.gpr[5] = 0;
        s.set_xer_ov(true);
        let mut effects = Vec::new();
        exec_with_mem(
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
        assert_eq!(s.cr_field(0), 0b0001);
    }

    #[test]
    fn lvlx_full_overlap_forwards_without_yielding() {
        // Sanity: when the buffer covers the full 16-byte line, the
        // load proceeds without yielding.
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000;
        s.gpr[2] = 0;
        let mut effects = Vec::new();
        let mut store_buf = StoreBuffer::new();
        let val = 0xAABB_CCDD_EEFF_0011_2233_4455_6677_8899u128;
        assert!(store_buf.insert(0x1000, 16, val));
        let v = execute(
            &PpuInstruction::Lvlx {
                vt: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            UnitId::new(0),
            &[(0, &[0u8; 0x2000])],
            &mut effects,
            &mut store_buf,
        );
        assert_eq!(v, ExecuteVerdict::Continue);
        assert_eq!(s.vr[3], val);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "lwzu invalid form")]
    fn lwzu_with_ra_zero_panics_in_debug() {
        let mut s = PpuState::new();
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lwzu {
                rt: 3,
                ra: 0, // invalid: RA=0 has no base register to update
                imm: 0,
            },
            &mut s,
            0,
            &[0u8; 0x100],
            &mut effects,
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "lwzu invalid form")]
    fn lwzu_with_ra_eq_rt_panics_in_debug() {
        let mut s = PpuState::new();
        s.gpr[3] = 0x10;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lwzu {
                rt: 3,
                ra: 3, // invalid: EA-write to RA would clobber RT
                imm: 0,
            },
            &mut s,
            0,
            &[0u8; 0x100],
            &mut effects,
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "stwu invalid form")]
    fn stwu_with_ra_zero_panics_in_debug() {
        let mut s = PpuState::new();
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stwu {
                rs: 3,
                ra: 0,
                imm: 0,
            },
            &mut s,
            0,
            &[0u8; 0x100],
            &mut effects,
        );
    }
}
