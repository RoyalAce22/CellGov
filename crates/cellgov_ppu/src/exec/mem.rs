//! Memory dispatch: integer / atomic / vector / floating-point loads
//! and stores, plus `dcbz`. Paths share the `load_ze` / `load_se` /
//! `buffer_store` / `load_slice` helpers from the parent module so the
//! reservation clear-sweep stays consistent across them.

use crate::exec::memory_helpers::{buffer_store, load_se, load_slice, load_ze};
use crate::exec::{ExecuteVerdict, PpuFault};
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_sync::ReservedLine;
use cellgov_time::GuestTicks;

use cellgov_ps3_abi::hardware::DCBZ_BLOCK_BYTES;

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
        // [PPC-Book1 p:34 s:3.3] Load Byte and Zero (lbz, D-form): byte at EA -> RT[56:63], RT[0:55]=0.
        // [PPC-Book1 p:35 s:3.3] Load Halfword and Zero (lhz, D-form): zero-extend halfword to RT.
        // [PPC-Book1 p:36 s:3.3] Load Halfword Algebraic (lha, D-form): sign-extend halfword to RT.
        // [PPC-Book1 p:38 s:3.3] Load Word Algebraic (lwa, DS-form): sign-extend word to RT.
        // [PPC-Book1 p:39 s:3.3] Load Doubleword (ld, DS-form): MEM(EA,8) -> RT.
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
        // [PPC-Book1 p:36 s:3.3] lhau D-form: sign-extend halfword to RT; RA = EA. Requires RA != 0 && RA != RT.
        PpuInstruction::Lhau { rt, ra, imm } => {
            debug_assert_load_with_update("lhau", ra, rt);
            let ea = state.ea_d_form(ra, imm);
            match load_se(region_views, store_buf, ea, 2) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    state.gpr[ra as usize] = ea;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
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
                match load_ze(region_views, store_buf, ea, 4) {
                    Ok(val) => {
                        state.gpr[r] = val;
                        ea = ea.wrapping_add(4);
                    }
                    Err(ea) => return ExecuteVerdict::MemFault(ea),
                }
            }
            ExecuteVerdict::Continue
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
            let ea = state.ea_d_form(ra, imm);
            match load_se(region_views, store_buf, ea, 4) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        // [PPC-Book1 p:34 s:3.3] X-form indexed load variants (lbzx/lhzx/lwzx/ldx): EA = (RA|0)+(RB).
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
        // [PPC-Book1 p:34 s:3.3.1] X-form indexed loads with update: EA = (RA|0)+(RB), then RA = EA.
        PpuInstruction::Lwzux { rt, ra, rb } => {
            debug_assert_load_with_update("lwzux", ra, rt);
            let ea = state.ea_x_form(ra, rb);
            match load_ze(region_views, store_buf, ea, 4) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    state.gpr[ra as usize] = ea;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Lbzux { rt, ra, rb } => {
            debug_assert_load_with_update("lbzux", ra, rt);
            let ea = state.ea_x_form(ra, rb);
            match load_ze(region_views, store_buf, ea, 1) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    state.gpr[ra as usize] = ea;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Lhzux { rt, ra, rb } => {
            debug_assert_load_with_update("lhzux", ra, rt);
            let ea = state.ea_x_form(ra, rb);
            match load_ze(region_views, store_buf, ea, 2) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    state.gpr[ra as usize] = ea;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Ldux { rt, ra, rb } => {
            debug_assert_load_with_update("ldux", ra, rt);
            let ea = state.ea_x_form(ra, rb);
            match load_ze(region_views, store_buf, ea, 8) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    state.gpr[ra as usize] = ea;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        // [PPC-Book1 p:36 s:3.3] lhax / lhaux: load halfword algebraic (sign-extend 16->64).
        PpuInstruction::Lhax { rt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            match load_se(region_views, store_buf, ea, 2) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Lhaux { rt, ra, rb } => {
            debug_assert_load_with_update("lhaux", ra, rt);
            let ea = state.ea_x_form(ra, rb);
            match load_se(region_views, store_buf, ea, 2) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    state.gpr[ra as usize] = ea;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        // [PPC-Book1 p:38 s:3.3] lwax / lwaux: load word algebraic (sign-extend 32->64).
        PpuInstruction::Lwax { rt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            match load_se(region_views, store_buf, ea, 4) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Lwaux { rt, ra, rb } => {
            debug_assert_load_with_update("lwaux", ra, rt);
            let ea = state.ea_x_form(ra, rb);
            match load_se(region_views, store_buf, ea, 4) {
                Ok(val) => {
                    state.gpr[rt as usize] = val;
                    state.gpr[ra as usize] = ea;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }

        // Integer stores
        // [PPC-Book1 p:40 s:3.3.3] Store Byte (stb, D-form): RS[56:63] -> MEM(EA,1).
        // [PPC-Book1 p:41 s:3.3.3] Store Halfword (sth, D-form): RS[48:63] -> MEM(EA,2).
        // [PPC-Book1 p:42 s:3.3.3] Store Word (stw/stwx/stwu, D/X-form): RS[32:63] -> MEM(EA,4).
        // [PPC-Book1 p:43 s:3.3.3] Store Doubleword (std/stdx, DS/X-form): RS -> MEM(EA,8).
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
            if verdict.allows_writeback() {
                state.gpr[ra as usize] = ea;
            }
            verdict
        }
        PpuInstruction::Stbx { rs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            buffer_store(store_buf, state, ea, 1, state.gpr[rs as usize])
        }
        // [PPC-Book1 p:41 s:3.3.3] sthx X-form: low halfword of RS -> MEM(EA, 2).
        PpuInstruction::Sthx { rs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            buffer_store(store_buf, state, ea, 2, state.gpr[rs as usize])
        }
        // [PPC-Book1 p:41 s:3.3.3] sthux X-form with update; RA != 0.
        PpuInstruction::Sthux { rs, ra, rb } => {
            debug_assert_store_with_update("sthux", ra);
            let ea = state.ea_x_form(ra, rb);
            let v = buffer_store(store_buf, state, ea, 2, state.gpr[rs as usize]);
            if v.allows_writeback() {
                state.gpr[ra as usize] = ea;
            }
            v
        }
        // [PPC-Book1 p:42 s:3.3.3] stwux X-form with update; RA != 0.
        PpuInstruction::Stwux { rs, ra, rb } => {
            debug_assert_store_with_update("stwux", ra);
            let ea = state.ea_x_form(ra, rb);
            let v = buffer_store(store_buf, state, ea, 4, state.gpr[rs as usize]);
            if v.allows_writeback() {
                state.gpr[ra as usize] = ea;
            }
            v
        }
        // [PPC-Book1 p:40 s:3.3.3] stbux X-form with update; RA != 0.
        PpuInstruction::Stbux { rs, ra, rb } => {
            debug_assert_store_with_update("stbux", ra);
            let ea = state.ea_x_form(ra, rb);
            let v = buffer_store(store_buf, state, ea, 1, state.gpr[rs as usize]);
            if v.allows_writeback() {
                state.gpr[ra as usize] = ea;
            }
            v
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

        // Byte-reverse indexed loads and stores
        // [PPC-Book1 p:50 s:3.3.4] lwbrx / lhbrx: load size N, low N bytes byte-reversed, zero-extended into RT.
        // [PPC-Book1 p:51 s:3.3.4] ldbrx / stwbrx / sthbrx: doubleword load / word + halfword store with byte reversal.
        // [CBE-Handbook p:734 s:A.2.1] sdbrx (CG name): low-64 byte-reverse store.
        PpuInstruction::Ldbrx { rt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            match load_ze(region_views, store_buf, ea, 8) {
                Ok(val) => {
                    state.gpr[rt as usize] = val.swap_bytes();
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Lwbrx { rt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            match load_ze(region_views, store_buf, ea, 4) {
                Ok(val) => {
                    state.gpr[rt as usize] = (val as u32).swap_bytes() as u64;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Lhbrx { rt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            match load_ze(region_views, store_buf, ea, 2) {
                Ok(val) => {
                    state.gpr[rt as usize] = (val as u16).swap_bytes() as u64;
                    ExecuteVerdict::Continue
                }
                Err(ea) => ExecuteVerdict::MemFault(ea),
            }
        }
        PpuInstruction::Sdbrx { rs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            let val = state.gpr[rs as usize].swap_bytes();
            buffer_store(store_buf, state, ea, 8, val)
        }
        PpuInstruction::Stwbrx { rs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            let val = (state.gpr[rs as usize] as u32).swap_bytes() as u64;
            buffer_store(store_buf, state, ea, 4, val)
        }
        PpuInstruction::Sthbrx { rs, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            let val = (state.gpr[rs as usize] as u16).swap_bytes() as u64;
            buffer_store(store_buf, state, ea, 2, val)
        }

        // Atomic load-reserve / store-conditional
        // [PPC-Book2 p:24 s:3.3] lwarx/ldarx: load + set RESERVE, RESERVE_ADDR = real_addr(EA); EA must be naturally aligned.
        PpuInstruction::Ldarx { rt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            state.ldarx_executed = state.ldarx_executed.wrapping_add(1);
            if ea & 7 != 0 {
                return ExecuteVerdict::Fault(PpuFault::AlignmentInterrupt(ea));
            }
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
        // [PPC-Book2 p:25 s:3.3] stwcx./stdcx.: if RESERVE && RESERVE_ADDR == real_addr(EA) then store + CR0 = 0b00||1||XER[SO], else CR0 = 0b00||0||XER[SO]; reservation cleared.
        PpuInstruction::Stdcx { rs, ra, rb } => {
            // Local reservation is authoritative: cross-unit clears
            // happen at step start; same-unit overlap clears in
            // `buffer_store`.
            let ea = state.ea_x_form(ra, rb);
            state.stdcx_executed = state.stdcx_executed.wrapping_add(1);
            if ea & 7 != 0 {
                return ExecuteVerdict::Fault(PpuFault::AlignmentInterrupt(ea));
            }
            let success = match state.reservation {
                Some(line) => line.addr() == ReservedLine::containing(ea).addr(),
                None => false,
            };
            // [PPC-Book2 p:25 s:3.3.2 Atomic Update Primitives] CR0 = 0b00 || n || XER[SO].
            let so = u8::from(state.xer_so());
            if success {
                state.set_cr_field(0, 0b0010 | so);
                let range = match ByteRange::new(GuestAddr::new(ea), 8) {
                    Some(r) => r,
                    None => {
                        return ExecuteVerdict::MemFault(cellgov_mem::MemError::Unmapped(
                            cellgov_mem::FaultContext {
                                addr: ea,
                                nearest_below: None,
                                nearest_above: None,
                            },
                        ));
                    }
                };
                let value = state.gpr[rs as usize];
                let bytes = value.to_be_bytes();
                // The commit pipeline stages every SharedWriteIntent
                // before ConditionalStore; draining the buffer here
                // would discard plain-store entries needed for
                // intra-batch load forwarding.
                effects.push(Effect::ConditionalStore {
                    range,
                    bytes: WritePayload::from_slice(&bytes),
                    ordering: PriorityClass::Normal,
                    source: unit_id,
                    source_time: GuestTicks::ZERO,
                });
                // Forwarding-only entry: same-step loads see the
                // committed bytes. Flush skips conditional entries.
                store_buf.insert_conditional(ea, 8, value as u128);
            } else {
                state.set_cr_field(0, so);
            }
            state.reservation = None;
            ExecuteVerdict::Continue
        }
        PpuInstruction::Lwarx { rt, ra, rb } => {
            let ea = state.ea_x_form(ra, rb);
            state.lwarx_executed = state.lwarx_executed.wrapping_add(1);
            if ea & 3 != 0 {
                return ExecuteVerdict::Fault(PpuFault::AlignmentInterrupt(ea));
            }
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
            // See `Stdcx` for the reservation / flush / forward-entry contract.
            let ea = state.ea_x_form(ra, rb);
            state.stwcx_executed = state.stwcx_executed.wrapping_add(1);
            if ea & 3 != 0 {
                return ExecuteVerdict::Fault(PpuFault::AlignmentInterrupt(ea));
            }
            let success = match state.reservation {
                Some(line) => line.addr() == ReservedLine::containing(ea).addr(),
                None => false,
            };
            let so = u8::from(state.xer_so());
            if success {
                state.set_cr_field(0, 0b0010 | so);
                let range = match ByteRange::new(GuestAddr::new(ea), 4) {
                    Some(r) => r,
                    None => {
                        return ExecuteVerdict::MemFault(cellgov_mem::MemError::Unmapped(
                            cellgov_mem::FaultContext {
                                addr: ea,
                                nearest_below: None,
                                nearest_above: None,
                            },
                        ));
                    }
                };
                let value32 = state.gpr[rs as usize] as u32;
                let bytes = value32.to_be_bytes();
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
            state.vr[vt as usize] = if shift == 0 { val } else { val << shift };
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
            state.vr[vt as usize] = if lo == 0 {
                0
            } else {
                val >> ((16 - lo) * 8) as u32
            };
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
            state.vr[vt as usize] = val;
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
            state.vr[vt as usize] = val;
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:6-21 s:6.2] lvsl: VRT[i] = sh + i for i in 0..16, where sh = EA[60:63].
        // Memory is NOT read; the result is a permute control vector derived from the low 4 bits of EA.
        PpuInstruction::Lvsl { vt, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let sh = (base.wrapping_add(state.gpr[rb as usize]) & 0xF) as u8;
            let mut bytes = [0u8; 16];
            for (i, b) in bytes.iter_mut().enumerate() {
                *b = sh.wrapping_add(i as u8);
            }
            state.vr[vt as usize] = u128::from_be_bytes(bytes);
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:6-22 s:6.2] lvsr: VRT[i] = 16 + i - sh for i in 0..16, where sh = EA[60:63].
        // Memory is NOT read; symmetric companion to lvsl.
        PpuInstruction::Lvsr { vt, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let sh = (base.wrapping_add(state.gpr[rb as usize]) & 0xF) as u8;
            let mut bytes = [0u8; 16];
            for (i, b) in bytes.iter_mut().enumerate() {
                *b = 16u8.wrapping_add(i as u8).wrapping_sub(sh);
            }
            state.vr[vt as usize] = u128::from_be_bytes(bytes);
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:6-15 s:6.2] lvebx: byte load at EA into byte position (EA & 0xF) of VRT.
        // Other byte lanes are spec-undefined; we preserve them.
        PpuInstruction::Lvebx { vt, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]);
            let m = (ea & 0xF) as usize;
            let byte = match load_ze(region_views, store_buf, ea, 1) {
                Ok(v) => v as u8,
                Err(e) => return ExecuteVerdict::MemFault(e),
            };
            let mut bytes = state.vr[vt as usize].to_be_bytes();
            bytes[m] = byte;
            state.vr[vt as usize] = u128::from_be_bytes(bytes);
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:6-16 s:6.2] lvehx: halfword load at (EA & ~1) into halfword position
        // ((EA & 0xE) / 2) of VRT; other lanes preserved (spec-undefined).
        PpuInstruction::Lvehx { vt, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]) & !1u64;
            let m = (ea & 0xF) as usize;
            let val = match load_ze(region_views, store_buf, ea, 2) {
                Ok(v) => v as u16,
                Err(e) => return ExecuteVerdict::MemFault(e),
            };
            let mut bytes = state.vr[vt as usize].to_be_bytes();
            let hb = val.to_be_bytes();
            bytes[m] = hb[0];
            bytes[m + 1] = hb[1];
            state.vr[vt as usize] = u128::from_be_bytes(bytes);
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:6-17 s:6.2] lvewx: word load at (EA & ~3) into word position
        // ((EA & 0xC) / 4) of VRT; other lanes preserved (spec-undefined).
        PpuInstruction::Lvewx { vt, ra, rb } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            let ea = base.wrapping_add(state.gpr[rb as usize]) & !3u64;
            let m = (ea & 0xF) as usize;
            let val = match load_ze(region_views, store_buf, ea, 4) {
                Ok(v) => v as u32,
                Err(e) => return ExecuteVerdict::MemFault(e),
            };
            let mut bytes = state.vr[vt as usize].to_be_bytes();
            let wb = val.to_be_bytes();
            bytes[m..m + 4].copy_from_slice(&wb);
            state.vr[vt as usize] = u128::from_be_bytes(bytes);
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

        // Floating-point loads / stores
        // [PPC-Book1 p:104 s:4.6] Load Floating-Point Single (lfs, D-form): single -> double via DOUBLE() into FRT.
        // [PPC-Book1 p:105 s:4.6] Load Floating-Point Double (lfd, D-form): MEM(EA,8) -> FRT.
        // [PPC-Book1 p:107 s:4.6] Store Floating-Point Single (stfs, D-form): SINGLE(FRS) -> MEM(EA,4).
        // [PPC-Book1 p:108 s:4.6] Store Floating-Point Double (stfd, D-form): FRS -> MEM(EA,8).
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
        // [PPC-Book1 p:104 s:4.6.2] lfsu D-form: lfs with EA written to RA. Requires RA != 0.
        PpuInstruction::Lfsu { frt, ra, imm } => {
            debug_assert_store_with_update("lfsu", ra);
            let ea = state.ea_d_form(ra, imm);
            match load_ze(region_views, store_buf, ea, 4) {
                Ok(bits) => {
                    state.fpr[frt as usize] = double_word(bits as u32);
                    state.gpr[ra as usize] = ea;
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
        // [PPC-Book1 p:105 s:4.6.2] lfdu D-form: lfd with EA written to RA. Requires RA != 0.
        PpuInstruction::Lfdu { frt, ra, imm } => {
            debug_assert_store_with_update("lfdu", ra);
            let ea = state.ea_d_form(ra, imm);
            match load_ze(region_views, store_buf, ea, 8) {
                Ok(bits) => {
                    state.fpr[frt as usize] = bits;
                    state.gpr[ra as usize] = ea;
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
        // [PPC-Book1 p:109 s:4.6] Store Floating-Point as Integer Word Indexed (stfiwx): FRS[32:63] -> MEM(EA,4) without conversion.
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
        // [PPC-Book1 p:104 s:4.6] lfsx / lfsux: X-form single-precision load with DOUBLE() conversion.
        // [PPC-Book1 p:105 s:4.6] lfdx / lfdux: X-form double-precision load.
        // [PPC-Book1 p:107 s:4.6] stfsx / stfsux: X-form single-precision store via SINGLE() conversion.
        // [PPC-Book1 p:108 s:4.6] stfdx / stfdux: X-form double-precision store.
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

/// Resolve a 16-byte aligned vector-line read with store-buffer overlay.
///
/// # Errors
///
/// Returns an `Unmapped` `MemError` when no region view covers the line.
fn read_aligned_16(
    aligned: u64,
    region_views: &[(u64, &[u8])],
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
    region_views: &[(u64, &[u8])],
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
    state.gpr[reg] = 0;
    for i in 0..n {
        let ea = base.wrapping_add(i as u64);
        let byte = match load_ze(region_views, store_buf, ea, 1) {
            Ok(v) => v as u8,
            Err(e) => return ExecuteVerdict::MemFault(e),
        };
        let shift = (3 - byte_idx) * 8;
        state.gpr[reg] |= (byte as u64) << shift;
        byte_idx += 1;
        if byte_idx == 4 && i + 1 < n {
            byte_idx = 0;
            reg = (reg + 1) % 32;
            state.gpr[reg] = 0;
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

fn debug_assert_load_with_update(insn: &str, ra: u8, rt: u8) {
    // [PPC-Book1 p:33 s:3.3.2] Load with update: invalid form when RA=0 or RA=RT.
    debug_assert!(ra != 0 && ra != rt, "{insn} invalid form: RA={ra}, RT={rt}");
}

#[inline]
#[track_caller]
fn debug_assert_store_with_update(insn: &str, ra: u8) {
    // [PPC-Book1 p:40 s:3.3.3] Store with update: invalid form when RA=0.
    debug_assert!(ra != 0, "{insn} invalid form: RA=0");
}

#[cfg(test)]
#[path = "tests/mem_tests.rs"]
mod tests;
