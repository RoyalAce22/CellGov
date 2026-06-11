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
mod tests {
    use super::*;
    use crate::exec::execute;
    use crate::exec::test_support::{exec_no_mem, exec_with_mem, uid};
    use cellgov_event::UnitId;
    use cellgov_sync::ReservedLine;

    #[test]
    fn lswi_packs_bytes_four_per_register_msb_first() {
        // 5 bytes from offset 0x10 -> RT=3, fills r3 fully + r4 partially.
        let mut mem = vec![0u8; 0x100];
        mem[0x10..0x15].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE]);
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        let mut effects = Vec::new();
        let result = exec_with_mem(
            &PpuInstruction::Lswi {
                rt: 3,
                ra: 1,
                nb: 5,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(result, ExecuteVerdict::Continue);
        assert_eq!(s.gpr[3], 0xAABB_CCDDu64);
        assert_eq!(s.gpr[4], 0xEE00_0000u64);
    }

    #[test]
    fn lswi_nb_zero_means_32_bytes_and_wraps_at_r31() {
        // RT=30 with NB=0 -> 32 bytes -> r30, r31, r0, r1 (wraps).
        let mut mem = vec![0u8; 0x100];
        for (i, slot) in mem.iter_mut().take(32).enumerate() {
            *slot = i as u8;
        }
        let mut s = PpuState::new();
        s.gpr[5] = 0;
        let mut effects = Vec::new();
        let result = exec_with_mem(
            &PpuInstruction::Lswi {
                rt: 30,
                ra: 5,
                nb: 0,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(result, ExecuteVerdict::Continue);
        // Bytes 0..3 -> r30, 4..7 -> r31, 8..11 -> r0, 12..15 -> r1, ...
        assert_eq!(s.gpr[30], 0x00010203);
        assert_eq!(s.gpr[31], 0x04050607);
        assert_eq!(s.gpr[0], 0x08090A0B);
        assert_eq!(s.gpr[1], 0x0C0D0E0F);
    }

    #[test]
    fn stswi_extracts_bytes_msb_first_from_consecutive_registers() {
        // 5 bytes from RS=3 (4 bytes from r3, 1 byte from r4 high).
        let mem = vec![0u8; 0x100];
        let mut s = PpuState::new();
        s.gpr[1] = 0x20;
        s.gpr[3] = 0xAABB_CCDDu64;
        s.gpr[4] = 0xEE00_0000u64;
        let mut effects = Vec::new();
        let result = exec_with_mem(
            &PpuInstruction::Stswi {
                rs: 3,
                ra: 1,
                nb: 5,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(result, ExecuteVerdict::Continue);
        // Inspect the 5 SharedWriteIntent effects via their inline bytes.
        let writes: Vec<(u64, u8)> = effects
            .iter()
            .filter_map(|e| match e {
                cellgov_effects::Effect::SharedWriteIntent { range, bytes, .. } => {
                    Some((range.start().raw(), bytes.bytes()[0]))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            writes,
            vec![
                (0x20, 0xAA),
                (0x21, 0xBB),
                (0x22, 0xCC),
                (0x23, 0xDD),
                (0x24, 0xEE),
            ]
        );
    }

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
        assert!(matches!(
            result,
            ExecuteVerdict::MemFault(cellgov_mem::MemError::Unmapped(ctx)) if ctx.addr == 0x1008
        ));
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
        // stw then lwa from same EA: forwards through StoreBuffer,
        // exercises size-aware sign extension (sub-8-byte forwards
        // leave high u64 bits zero, so naive i64 cast would mis-sign).
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
    fn ldarx_increments_counter_on_each_execution() {
        let mem = vec![0u8; 0x2000];
        let mut s = PpuState::new();
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x8;
        assert_eq!(s.ldarx_executed, 0);
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
        assert_eq!(s.ldarx_executed, 1);
        s.gpr[4] = 0x10;
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
        assert_eq!(s.ldarx_executed, 2);
    }

    #[test]
    fn lwarx_increments_counter_on_each_execution() {
        let mem = vec![0u8; 0x2000];
        let mut s = PpuState::new();
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x40;
        assert_eq!(s.lwarx_executed, 0);
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
        assert_eq!(s.lwarx_executed, 1);
        s.gpr[4] = 0x44;
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
        assert_eq!(s.lwarx_executed, 2);
    }

    #[test]
    fn stdcx_increments_counter_on_each_execution() {
        let mut s = PpuState::new();
        s.reservation = Some(ReservedLine::containing(0x1000));
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x0;
        s.gpr[5] = 0xCAFE_BABE_DEAD_BEEF;
        assert_eq!(s.stdcx_executed, 0);
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
        assert_eq!(s.stdcx_executed, 1);
        // Counter counts arm entries, not successful conditional stores.
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
        assert_eq!(s.stdcx_executed, 2);
    }

    #[test]
    fn stwcx_increments_counter_on_each_execution() {
        let mut s = PpuState::new();
        s.reservation = Some(ReservedLine::containing(0x1000));
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x0;
        s.gpr[5] = 0xCAFE_BABE;
        assert_eq!(s.stwcx_executed, 0);
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
        assert_eq!(s.stwcx_executed, 1);
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
        assert_eq!(s.stwcx_executed, 2);
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
    fn dcbz_increments_counter_on_each_execution() {
        // C-4 audit witness: dcbz_executed increments per Dcbz
        // arm entry. The MMIO-window debug_assert is evaluated
        // before the counter resolves; this proves silence is
        // non-vacuous when the counter is > 0.
        let base = 0x1000u64;
        let mem = vec![0u8; 384];
        let mut s = PpuState::new();
        assert_eq!(s.dcbz_executed, 0);
        s.gpr[3] = 0;
        s.gpr[4] = 0x1000;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Dcbz { ra: 3, rb: 4 },
            &mut s,
            base,
            &mem,
            &mut effects,
        );
        assert_eq!(s.dcbz_executed, 1);
        exec_with_mem(
            &PpuInstruction::Dcbz { ra: 3, rb: 4 },
            &mut s,
            base,
            &mem,
            &mut effects,
        );
        assert_eq!(s.dcbz_executed, 2);
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
        // [PPC-Book2 p:25 s:3.3.2 Atomic Update Primitives] stwcx
        // CR0 = 0b00 || n || XER[SO]. Earlier code zeroed the SO bit
        // unconditionally.
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

    // -----------------------------------------------------------------
    // Integer D-form loads
    // -----------------------------------------------------------------

    #[test]
    fn lbz_loads_byte_zero_extended() {
        let mut mem = vec![0u8; 0x100];
        mem[0x20] = 0xA5;
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
        let mut effects = Vec::new();
        let v = exec_with_mem(
            &PpuInstruction::Lbz {
                rt: 3,
                ra: 1,
                imm: 0x10,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(v, ExecuteVerdict::Continue);
        assert_eq!(s.gpr[3], 0xA5);
    }

    #[test]
    fn lhz_loads_halfword_zero_extended_big_endian() {
        let mut mem = vec![0u8; 0x100];
        mem[0x10..0x12].copy_from_slice(&[0xBE, 0xEF]);
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lhz {
                rt: 3,
                ra: 1,
                imm: 0,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3], 0xBEEF);
    }

    #[test]
    fn ld_loads_doubleword_big_endian() {
        let mut mem = vec![0u8; 0x100];
        mem[0x10..0x18].copy_from_slice(&0x0123_4567_89AB_CDEFu64.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Ld {
                rt: 3,
                ra: 1,
                imm: 0,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3], 0x0123_4567_89AB_CDEF);
    }

    #[test]
    fn lhau_sign_extends_and_writes_back_ra() {
        let mut mem = vec![0u8; 0x100];
        mem[0x12..0x14].copy_from_slice(&0x8000u16.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lhau {
                rt: 3,
                ra: 1,
                imm: 2,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3] as i64, i16::MIN as i64);
        assert_eq!(s.gpr[1], 0x12);
    }

    #[test]
    fn lwzu_loads_word_and_writes_back_ra() {
        let mut mem = vec![0u8; 0x100];
        mem[0x14..0x18].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lwzu {
                rt: 3,
                ra: 1,
                imm: 4,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3], 0xDEAD_BEEF);
        assert_eq!(s.gpr[1], 0x14);
    }

    #[test]
    fn lbzu_loads_byte_and_writes_back_ra() {
        let mut mem = vec![0u8; 0x100];
        mem[0x11] = 0x7E;
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lbzu {
                rt: 3,
                ra: 1,
                imm: 1,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3], 0x7E);
        assert_eq!(s.gpr[1], 0x11);
    }

    #[test]
    fn lmw_loads_consecutive_words_until_r31() {
        let mut mem = vec![0u8; 0x100];
        for r in 0..3u32 {
            let off = 0x10 + (r as usize) * 4;
            mem[off..off + 4].copy_from_slice(&(0xAABB_0000u32 + r).to_be_bytes());
        }
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        let mut effects = Vec::new();
        // RT=29 -> loads r29, r30, r31 (three words).
        let v = exec_with_mem(
            &PpuInstruction::Lmw {
                rt: 29,
                ra: 1,
                imm: 0,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(v, ExecuteVerdict::Continue);
        assert_eq!(s.gpr[29], 0xAABB_0000);
        assert_eq!(s.gpr[30], 0xAABB_0001);
        assert_eq!(s.gpr[31], 0xAABB_0002);
    }

    #[test]
    fn lfault_lhau_does_not_update_ra() {
        let mem = vec![0u8; 0x100];
        let mut s = PpuState::new();
        s.gpr[1] = 0x1000_0000;
        let original = s.gpr[1];
        let mut effects = Vec::new();
        let v = exec_with_mem(
            &PpuInstruction::Lhau {
                rt: 3,
                ra: 1,
                imm: 0,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert!(matches!(v, ExecuteVerdict::MemFault(_)));
        assert_eq!(s.gpr[1], original);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "lhau invalid form")]
    fn lhau_with_ra_zero_panics_in_debug() {
        let mut s = PpuState::new();
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lhau {
                rt: 3,
                ra: 0,
                imm: 0,
            },
            &mut s,
            0,
            &[0u8; 0x100],
            &mut effects,
        );
    }

    // -----------------------------------------------------------------
    // Integer X-form loads
    // -----------------------------------------------------------------

    #[test]
    fn lwzx_loads_word_at_ra_plus_rb() {
        let mut mem = vec![0u8; 0x100];
        mem[0x20..0x24].copy_from_slice(&0xCAFE_BABEu32.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        s.gpr[2] = 0x10;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lwzx {
                rt: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3], 0xCAFE_BABE);
    }

    #[test]
    fn lbzx_loads_byte_zero_extended() {
        let mut mem = vec![0u8; 0x100];
        mem[0x30] = 0x42;
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        s.gpr[2] = 0x20;
        s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lbzx {
                rt: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3], 0x42);
    }

    #[test]
    fn lhzx_loads_halfword_zero_extended() {
        let mut mem = vec![0u8; 0x100];
        mem[0x20..0x22].copy_from_slice(&0xABCDu16.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        s.gpr[2] = 0x10;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lhzx {
                rt: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3], 0xABCD);
    }

    #[test]
    fn ldx_loads_doubleword_with_ra_zero_ignored() {
        // ea_x_form with RA=0 uses literal 0, not gpr[0].
        let mut mem = vec![0u8; 0x100];
        mem[0x18..0x20].copy_from_slice(&0xCAFE_F00D_DEAD_BEEFu64.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[0] = 0xDEAD; // must be ignored
        s.gpr[2] = 0x18;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Ldx {
                rt: 3,
                ra: 0,
                rb: 2,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3], 0xCAFE_F00D_DEAD_BEEF);
    }

    #[test]
    fn lhax_sign_extends_halfword() {
        let mut mem = vec![0u8; 0x100];
        mem[0x20..0x22].copy_from_slice(&0xFF80u16.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        s.gpr[2] = 0x10;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lhax {
                rt: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3] as i64, -128);
    }

    #[test]
    fn lwax_sign_extends_word() {
        let mut mem = vec![0u8; 0x100];
        mem[0x20..0x24].copy_from_slice(&0xFFFF_FFFEu32.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        s.gpr[2] = 0x10;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lwax {
                rt: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3] as i64, -2);
    }

    #[test]
    fn lwzux_loads_and_writes_back_ra_only_on_success() {
        let mut mem = vec![0u8; 0x100];
        mem[0x20..0x24].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[4] = 0x10;
        s.gpr[5] = 0x10;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lwzux {
                rt: 3,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3], 0xDEAD_BEEF);
        assert_eq!(s.gpr[4], 0x20);
    }

    #[test]
    fn lwzux_fault_leaves_ra_unchanged() {
        let mem = vec![0u8; 0x40];
        let mut s = PpuState::new();
        s.gpr[4] = 0x1000_0000;
        s.gpr[5] = 0;
        let original = s.gpr[4];
        let mut effects = Vec::new();
        let v = exec_with_mem(
            &PpuInstruction::Lwzux {
                rt: 3,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert!(matches!(v, ExecuteVerdict::MemFault(_)));
        assert_eq!(s.gpr[4], original);
    }

    #[test]
    fn lbzux_loads_byte_and_writes_back_ra() {
        let mut mem = vec![0u8; 0x100];
        mem[0x21] = 0x99;
        let mut s = PpuState::new();
        s.gpr[4] = 0x10;
        s.gpr[5] = 0x11;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lbzux {
                rt: 3,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3], 0x99);
        assert_eq!(s.gpr[4], 0x21);
    }

    #[test]
    fn lhzux_loads_halfword_and_writes_back_ra() {
        let mut mem = vec![0u8; 0x100];
        mem[0x20..0x22].copy_from_slice(&0xC0DEu16.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[4] = 0x10;
        s.gpr[5] = 0x10;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lhzux {
                rt: 3,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3], 0xC0DE);
        assert_eq!(s.gpr[4], 0x20);
    }

    #[test]
    fn ldux_loads_doubleword_and_writes_back_ra() {
        let mut mem = vec![0u8; 0x100];
        mem[0x18..0x20].copy_from_slice(&0xDEAD_BEEF_CAFE_BABEu64.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[4] = 0x10;
        s.gpr[5] = 0x8;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Ldux {
                rt: 3,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3], 0xDEAD_BEEF_CAFE_BABE);
        assert_eq!(s.gpr[4], 0x18);
    }

    #[test]
    fn lhaux_sign_extends_and_writes_back_ra() {
        let mut mem = vec![0u8; 0x100];
        mem[0x20..0x22].copy_from_slice(&0xFFFFu16.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[4] = 0x10;
        s.gpr[5] = 0x10;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lhaux {
                rt: 3,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3] as i64, -1);
        assert_eq!(s.gpr[4], 0x20);
    }

    #[test]
    fn lwaux_sign_extends_and_writes_back_ra() {
        let mut mem = vec![0u8; 0x100];
        mem[0x20..0x24].copy_from_slice(&0x8000_0000u32.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[4] = 0x10;
        s.gpr[5] = 0x10;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lwaux {
                rt: 3,
                ra: 4,
                rb: 5,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3], 0xFFFF_FFFF_8000_0000);
        assert_eq!(s.gpr[4], 0x20);
    }

    // -----------------------------------------------------------------
    // Integer stores
    // -----------------------------------------------------------------

    #[test]
    fn std_emits_8_byte_store_big_endian() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[5] = 0x0123_4567_89AB_CDEF;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Std {
                rs: 5,
                ra: 1,
                imm: 0x10,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x110);
                assert_eq!(range.length(), 8);
                assert_eq!(bytes.bytes(), &0x0123_4567_89AB_CDEFu64.to_be_bytes());
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn stdu_updates_ra_and_emits_8_byte_store() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[5] = 0xCAFE_F00D_DEAD_BEEF;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stdu {
                rs: 5,
                ra: 1,
                imm: -8,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        assert_eq!(s.gpr[1], 0xF8);
        match &effects[0] {
            Effect::SharedWriteIntent { range, .. } => assert_eq!(range.start().raw(), 0xF8),
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn stmw_emits_words_starting_at_rs_through_r31() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[29] = 0x1111_2222;
        s.gpr[30] = 0x3333_4444;
        s.gpr[31] = 0x5555_6666;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stmw {
                rs: 29,
                ra: 1,
                imm: 0,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        let writes: Vec<(u64, [u8; 4])> = effects
            .iter()
            .filter_map(|e| match e {
                Effect::SharedWriteIntent { range, bytes, .. } => {
                    let b: [u8; 4] = bytes.bytes().try_into().ok()?;
                    Some((range.start().raw(), b))
                }
                _ => None,
            })
            .collect();
        assert_eq!(writes.len(), 3);
        assert_eq!(writes[0], (0x100, 0x1111_2222u32.to_be_bytes()));
        assert_eq!(writes[1], (0x104, 0x3333_4444u32.to_be_bytes()));
        assert_eq!(writes[2], (0x108, 0x5555_6666u32.to_be_bytes()));
    }

    #[test]
    fn stwx_emits_word_store_at_ra_plus_rb() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0x10;
        s.gpr[5] = 0xDEAD_BEEF;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stwx {
                rs: 5,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x110);
                assert_eq!(bytes.bytes(), &0xDEAD_BEEFu32.to_be_bytes());
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn stdx_emits_8_byte_store_at_ra_plus_rb() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0x8;
        s.gpr[5] = 0x0011_2233_4455_6677;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stdx {
                rs: 5,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x108);
                assert_eq!(bytes.bytes(), &0x0011_2233_4455_6677u64.to_be_bytes());
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn stbx_emits_low_byte_store() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0x4;
        s.gpr[5] = 0xFFFF_FFFF_FFFF_FF42;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stbx {
                rs: 5,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x104);
                assert_eq!(bytes.bytes(), &[0x42]);
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn sthx_emits_low_halfword_store_big_endian() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0x2;
        s.gpr[5] = 0xFFFF_FFFF_FFFF_BEEF;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Sthx {
                rs: 5,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x102);
                assert_eq!(bytes.bytes(), &0xBEEFu16.to_be_bytes());
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn sthux_writes_back_ra_only_on_success() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0x4;
        s.gpr[5] = 0xCAFE;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Sthux {
                rs: 5,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        assert_eq!(s.gpr[1], 0x104);
        match &effects[0] {
            Effect::SharedWriteIntent { range, .. } => assert_eq!(range.start().raw(), 0x104),
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn stwux_writes_back_ra_only_on_success() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0x8;
        s.gpr[5] = 0xDEAD_BEEF;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stwux {
                rs: 5,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        assert_eq!(s.gpr[1], 0x108);
    }

    #[test]
    fn stbux_writes_back_ra_only_on_success() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0x1;
        s.gpr[5] = 0x33;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stbux {
                rs: 5,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        assert_eq!(s.gpr[1], 0x101);
    }

    // -----------------------------------------------------------------
    // String load / store
    // -----------------------------------------------------------------

    #[test]
    fn lswx_uses_xer_tbc_byte_count() {
        let mut mem = vec![0u8; 0x100];
        mem[0x20..0x23].copy_from_slice(&[0x11, 0x22, 0x33]);
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        s.gpr[2] = 0x10;
        s.xer = 0x3; // TBC=3 bytes
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lswx {
                rt: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        // 3 bytes packed MSB-first into the low 32 bits of r3.
        assert_eq!(s.gpr[3], 0x1122_3300);
    }

    #[test]
    fn lswx_zero_byte_count_is_noop() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        s.gpr[2] = 0x10;
        s.gpr[3] = 0xDEAD_BEEF;
        s.xer = 0;
        let mut effects = Vec::new();
        let v = exec_with_mem(
            &PpuInstruction::Lswx {
                rt: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x100],
            &mut effects,
        );
        assert_eq!(v, ExecuteVerdict::Continue);
        // No write: r3 untouched.
        assert_eq!(s.gpr[3], 0xDEAD_BEEF);
    }

    #[test]
    fn stswx_uses_xer_tbc_byte_count() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        s.gpr[2] = 0x10;
        s.gpr[3] = 0xAABB_CCDDu64;
        s.xer = 0x2;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stswx {
                rs: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x100],
            &mut effects,
        );
        let writes: Vec<(u64, u8)> = effects
            .iter()
            .filter_map(|e| match e {
                Effect::SharedWriteIntent { range, bytes, .. } => {
                    Some((range.start().raw(), bytes.bytes()[0]))
                }
                _ => None,
            })
            .collect();
        assert_eq!(writes, vec![(0x20, 0xAA), (0x21, 0xBB)]);
    }

    #[test]
    fn stswi_nb_zero_means_32_bytes() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        for r in 0..8u32 {
            s.gpr[3 + r as usize] = (r as u64) * 0x0101_0101_0101_0101 + 0x1020_3040;
        }
        // To make the assertion simple, seed r3..r10 with known 32-bit words.
        for r in 0..8usize {
            s.gpr[3 + r] = (0xA0 + r as u64) << 24 | ((0xB0 + r as u64) << 16);
        }
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stswi {
                rs: 3,
                ra: 1,
                nb: 0,
            },
            &mut s,
            0,
            &[0u8; 0x100],
            &mut effects,
        );
        // NB=0 -> 32 bytes -> exactly 32 byte-stores.
        let count = effects
            .iter()
            .filter(|e| matches!(e, Effect::SharedWriteIntent { .. }))
            .count();
        assert_eq!(count, 32);
    }

    #[test]
    fn lswi_with_ra_zero_uses_literal_zero_base() {
        let mut mem = vec![0u8; 0x20];
        mem[0..4].copy_from_slice(&[0x01, 0x02, 0x03, 0x04]);
        let mut s = PpuState::new();
        s.gpr[0] = 0xDEAD; // must be ignored
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lswi {
                rt: 3,
                ra: 0,
                nb: 4,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3], 0x0102_0304);
    }

    // -----------------------------------------------------------------
    // Byte-reverse loads / stores
    // -----------------------------------------------------------------

    #[test]
    fn ldbrx_reverses_8_bytes() {
        let mut mem = vec![0u8; 0x100];
        mem[0x10..0x18].copy_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        s.gpr[2] = 0;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Ldbrx {
                rt: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        // Reading BE -> 0x0102030405060708, swap_bytes -> 0x0807060504030201.
        assert_eq!(s.gpr[3], 0x0807_0605_0403_0201);
    }

    #[test]
    fn lwbrx_reverses_low_4_bytes_and_zero_extends() {
        let mut mem = vec![0u8; 0x100];
        mem[0x10..0x14].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        s.gpr[2] = 0;
        s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lwbrx {
                rt: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3], 0x0000_0000_DDCC_BBAA);
    }

    #[test]
    fn lhbrx_reverses_halfword_and_zero_extends() {
        let mut mem = vec![0u8; 0x100];
        mem[0x10..0x12].copy_from_slice(&[0x12, 0x34]);
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        s.gpr[2] = 0;
        s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lhbrx {
                rt: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.gpr[3], 0x0000_0000_0000_3412);
    }

    #[test]
    fn sdbrx_emits_byte_reversed_8_byte_store() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0;
        s.gpr[5] = 0x0102_0304_0506_0708;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Sdbrx {
                rs: 5,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x100);
                assert_eq!(bytes.bytes(), &0x0807_0605_0403_0201u64.to_be_bytes());
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn stwbrx_emits_byte_reversed_low_4_bytes() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0;
        s.gpr[5] = 0xFFFF_FFFF_AABB_CCDD;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stwbrx {
                rs: 5,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x100);
                assert_eq!(bytes.bytes(), &0xDDCC_BBAAu32.to_be_bytes());
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn sthbrx_emits_byte_reversed_low_halfword() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0;
        s.gpr[5] = 0xFFFF_FFFF_FFFF_1234;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Sthbrx {
                rs: 5,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x100);
                assert_eq!(bytes.bytes(), &0x3412u16.to_be_bytes());
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // AltiVec aligned vector loads
    // -----------------------------------------------------------------

    #[test]
    fn lvx_aligns_ea_down_to_16_byte_boundary() {
        let mut mem = vec![0u8; 0x200];
        let pattern: [u8; 16] = [
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D,
            0x1E, 0x1F,
        ];
        mem[0x100..0x110].copy_from_slice(&pattern);
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0x0F; // EA = 0x10F -> aligned 0x100
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lvx {
                vt: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.vr[3], u128::from_be_bytes(pattern));
    }

    #[test]
    fn lvxl_matches_lvx_semantics() {
        // lvxl is lvx with an ignored cache hint; same bytes -> same VR.
        let mut mem = vec![0u8; 0x200];
        let pattern: [u8; 16] = [
            0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x2B, 0x2C, 0x2D,
            0x2E, 0x2F,
        ];
        mem[0x100..0x110].copy_from_slice(&pattern);
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lvxl {
                vt: 4,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.vr[4], u128::from_be_bytes(pattern));
    }

    #[test]
    fn lvsl_sh_zero_returns_identity_vector() {
        // sh=0 -> VRT = [0, 1, 2, ..., 15].
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0;
        let mut effects = Vec::new();
        exec_no_mem_or_load(&mut s, &mut effects, |s, e| {
            exec_with_mem(
                &PpuInstruction::Lvsl {
                    vt: 3,
                    ra: 1,
                    rb: 2,
                },
                s,
                0,
                &[0u8; 0x10],
                e,
            )
        });
        let expected_bytes: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        assert_eq!(s.vr[3], u128::from_be_bytes(expected_bytes));
    }

    #[test]
    fn lvsl_sh_nonzero_returns_shifted_identity() {
        // EA & 0xF = 3 -> VRT = [3, 4, 5, ..., 18].
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0x3;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lvsl {
                vt: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x10],
            &mut effects,
        );
        let mut expected = [0u8; 16];
        for (i, b) in expected.iter_mut().enumerate() {
            *b = 3 + i as u8;
        }
        assert_eq!(s.vr[3], u128::from_be_bytes(expected));
    }

    #[test]
    fn lvsr_sh_zero_returns_descending_from_16() {
        // sh=0 -> VRT = [16, 17, ..., 31] (wraps low bits of u8).
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lvsr {
                vt: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x10],
            &mut effects,
        );
        let mut expected = [0u8; 16];
        for (i, b) in expected.iter_mut().enumerate() {
            *b = 16u8.wrapping_add(i as u8);
        }
        assert_eq!(s.vr[3], u128::from_be_bytes(expected));
    }

    #[test]
    fn lvsr_sh_three_returns_companion_to_lvsl() {
        // sh=3 -> VRT[i] = 16 + i - 3 = 13 + i, for i in 0..16.
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0x3;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lvsr {
                vt: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x10],
            &mut effects,
        );
        let mut expected = [0u8; 16];
        for (i, b) in expected.iter_mut().enumerate() {
            *b = 13u8.wrapping_add(i as u8);
        }
        assert_eq!(s.vr[3], u128::from_be_bytes(expected));
    }

    #[test]
    fn lvebx_places_byte_in_be_lane_from_ea_low_nibble() {
        // EA & 0xF = 5 -> byte lands at byte[5] of the 16-byte BE view.
        let mut mem = vec![0u8; 0x100];
        mem[0x15] = 0x7E;
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        s.gpr[2] = 0x05;
        // Pre-seed VR with a sentinel so we can verify other lanes are
        // preserved (spec-undefined but our implementation preserves).
        s.vr[3] = 0xAAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAAu128;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lvebx {
                vt: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        let mut expected = [0xAAu8; 16];
        expected[5] = 0x7E;
        assert_eq!(s.vr[3], u128::from_be_bytes(expected));
    }

    #[test]
    fn lvehx_places_halfword_in_aligned_be_lane() {
        // EA = 0x14 -> after &!1 still 0x14; lane = 0x4.
        let mut mem = vec![0u8; 0x100];
        mem[0x14..0x16].copy_from_slice(&[0xBE, 0xEF]);
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        s.gpr[2] = 0x05; // EA = 0x15 -> aligned to 0x14
        s.vr[3] = 0;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lvehx {
                vt: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        let mut expected = [0u8; 16];
        expected[4] = 0xBE;
        expected[5] = 0xEF;
        assert_eq!(s.vr[3], u128::from_be_bytes(expected));
    }

    #[test]
    fn lvewx_places_word_in_aligned_be_lane() {
        // EA = 0x18 -> &!3 still 0x18; lane = 0x8.
        let mut mem = vec![0u8; 0x100];
        mem[0x18..0x1C].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        s.gpr[2] = 0x09; // EA = 0x19 -> aligned 0x18
        s.vr[3] = 0;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lvewx {
                vt: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        let mut expected = [0u8; 16];
        expected[8] = 0xDE;
        expected[9] = 0xAD;
        expected[10] = 0xBE;
        expected[11] = 0xEF;
        assert_eq!(s.vr[3], u128::from_be_bytes(expected));
    }

    #[test]
    fn lvlxl_matches_lvlx_semantics() {
        let mut mem = vec![0u8; 0x200];
        let pattern: [u8; 16] = [
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE,
            0xFF, 0x10,
        ];
        mem[0x100..0x110].copy_from_slice(&pattern);
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0x3;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lvlxl {
                vt: 5,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.vr[5], u128::from_be_bytes(pattern) << 24);
    }

    #[test]
    fn lvrxl_aligned_ea_returns_zero() {
        // Mirrors lvrx aligned: shift by 128 produces zero. NOTE the
        // implementation still issues the underlying line read before
        // computing the zero result; this test pins the architectural
        // outcome, not the buffer-side memoization shape.
        let mut mem = vec![0u8; 0x200];
        mem[0x100..0x110].copy_from_slice(&[0xFF; 16]);
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0;
        s.vr[5] = u128::MAX;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lvrxl {
                vt: 5,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.vr[5], 0);
    }

    // -----------------------------------------------------------------
    // AltiVec single-lane stores
    // -----------------------------------------------------------------

    #[test]
    fn stvebx_emits_byte_from_be_lane_ea_low_nibble() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0x5;
        // Lane 5 of the BE view holds the byte 0x55.
        let mut bytes = [0u8; 16];
        bytes[5] = 0x55;
        s.vr[3] = u128::from_be_bytes(bytes);
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stvebx {
                vs: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x105);
                assert_eq!(bytes.bytes(), &[0x55]);
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn stvehx_emits_halfword_from_aligned_be_lane() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0x5; // EA = 0x105 -> aligned 0x104, lane 4
        let mut bytes = [0u8; 16];
        bytes[4] = 0xBE;
        bytes[5] = 0xEF;
        s.vr[3] = u128::from_be_bytes(bytes);
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stvehx {
                vs: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x104);
                assert_eq!(bytes.bytes(), &[0xBE, 0xEF]);
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn stvewx_emits_word_from_aligned_be_lane() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0x9; // EA = 0x109 -> aligned 0x108, lane 8
        let mut bytes = [0u8; 16];
        bytes[8] = 0xDE;
        bytes[9] = 0xAD;
        bytes[10] = 0xBE;
        bytes[11] = 0xEF;
        s.vr[3] = u128::from_be_bytes(bytes);
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stvewx {
                vs: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x108);
                assert_eq!(bytes.bytes(), &[0xDE, 0xAD, 0xBE, 0xEF]);
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // AltiVec unaligned vector stores (stvlx/stvrx + l)
    // -----------------------------------------------------------------

    #[test]
    fn stvlx_writes_high_bytes_starting_at_ea() {
        // EA & 0xF = 3 -> count = 16 - 3 = 13 bytes from VS[0..13]
        // stored to [EA..EA+13].
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0x3;
        s.vr[3] = u128::from_be_bytes([
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D,
            0x1E, 0x1F,
        ]);
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stvlx {
                vs: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        let writes: Vec<(u64, u8)> = effects
            .iter()
            .filter_map(|e| match e {
                Effect::SharedWriteIntent { range, bytes, .. } => {
                    Some((range.start().raw(), bytes.bytes()[0]))
                }
                _ => None,
            })
            .collect();
        assert_eq!(writes.len(), 13);
        assert_eq!(writes[0], (0x103, 0x10));
        assert_eq!(writes[12], (0x10F, 0x1C));
    }

    #[test]
    fn stvrx_writes_low_bytes_to_aligned_line_below_ea() {
        // EA & 0xF = 3 -> 3 bytes from VS[13..16] -> [aligned..aligned+3].
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0x3;
        s.vr[3] = u128::from_be_bytes([
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D,
            0x1E, 0x1F,
        ]);
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stvrx {
                vs: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        let writes: Vec<(u64, u8)> = effects
            .iter()
            .filter_map(|e| match e {
                Effect::SharedWriteIntent { range, bytes, .. } => {
                    Some((range.start().raw(), bytes.bytes()[0]))
                }
                _ => None,
            })
            .collect();
        assert_eq!(writes, vec![(0x100, 0x1D), (0x101, 0x1E), (0x102, 0x1F)]);
    }

    #[test]
    fn stvrx_aligned_ea_is_noop() {
        // EA & 0xF = 0 -> m=0; no stores emitted.
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0;
        s.vr[3] = u128::MAX;
        let mut effects = Vec::new();
        let v = exec_with_mem(
            &PpuInstruction::Stvrx {
                vs: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        assert_eq!(v, ExecuteVerdict::Continue);
        assert!(effects.is_empty());
    }

    #[test]
    fn stvlxl_matches_stvlx_semantics() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0xC; // 16 - 12 = 4 bytes
        s.vr[3] = u128::from_be_bytes([
            0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD,
            0xAE, 0xAF,
        ]);
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stvlxl {
                vs: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        let writes: Vec<(u64, u8)> = effects
            .iter()
            .filter_map(|e| match e {
                Effect::SharedWriteIntent { range, bytes, .. } => {
                    Some((range.start().raw(), bytes.bytes()[0]))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            writes,
            vec![(0x10C, 0xA0), (0x10D, 0xA1), (0x10E, 0xA2), (0x10F, 0xA3),]
        );
    }

    #[test]
    fn stvrxl_matches_stvrx_semantics() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0x2; // 2 bytes from VS[14..16] -> [0x100, 0x101]
        s.vr[3] = u128::from_be_bytes([
            0xB0, 0xB1, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xBB, 0xBC, 0xBD,
            0xBE, 0xBF,
        ]);
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stvrxl {
                vs: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        let writes: Vec<(u64, u8)> = effects
            .iter()
            .filter_map(|e| match e {
                Effect::SharedWriteIntent { range, bytes, .. } => {
                    Some((range.start().raw(), bytes.bytes()[0]))
                }
                _ => None,
            })
            .collect();
        assert_eq!(writes, vec![(0x100, 0xBE), (0x101, 0xBF)]);
    }

    #[test]
    fn stvxl_emits_16_byte_store_in_two_halves() {
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.gpr[2] = 0;
        let val = 0x0011_2233_4455_6677_8899_AABB_CCDD_EEFFu128;
        s.vr[3] = val;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stvxl {
                vs: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        assert_eq!(effects.len(), 2);
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x100);
                assert_eq!(bytes.bytes(), &0x0011_2233_4455_6677u64.to_be_bytes());
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
        match &effects[1] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x108);
                assert_eq!(bytes.bytes(), &0x8899_AABB_CCDD_EEFFu64.to_be_bytes());
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // Floating-point D-form loads / stores
    // -----------------------------------------------------------------

    #[test]
    fn lfs_loads_single_and_converts_to_double() {
        // 2.0f single = 0x40000000 -> 2.0 double = 0x4000_0000_0000_0000.
        let mut mem = vec![0u8; 0x100];
        mem[0x10..0x14].copy_from_slice(&0x4000_0000u32.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lfs {
                frt: 5,
                ra: 1,
                imm: 0,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.fpr[5], 0x4000_0000_0000_0000);
    }

    #[test]
    fn lfsu_loads_and_writes_back_ra() {
        let mut mem = vec![0u8; 0x100];
        mem[0x14..0x18].copy_from_slice(&0x4040_0000u32.to_be_bytes()); // 3.0f
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lfsu {
                frt: 6,
                ra: 1,
                imm: 4,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.fpr[6], 0x4008_0000_0000_0000);
        assert_eq!(s.gpr[1], 0x14);
    }

    #[test]
    fn lfd_loads_8_byte_double_big_endian() {
        let mut mem = vec![0u8; 0x100];
        let bits = 0x4010_2030_4050_6070u64;
        mem[0x10..0x18].copy_from_slice(&bits.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lfd {
                frt: 5,
                ra: 1,
                imm: 0,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.fpr[5], bits);
    }

    #[test]
    fn lfdu_loads_double_and_writes_back_ra() {
        let mut mem = vec![0u8; 0x100];
        let bits = 0xDEAD_BEEF_CAFE_BABEu64;
        mem[0x18..0x20].copy_from_slice(&bits.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lfdu {
                frt: 5,
                ra: 1,
                imm: 8,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.fpr[5], bits);
        assert_eq!(s.gpr[1], 0x18);
    }

    #[test]
    fn stfs_round_converts_double_to_single() {
        // 1.5 double = 0x3FF8_0000_0000_0000 -> 1.5f single = 0x3FC00000.
        let mut s = PpuState::new();
        s.gpr[1] = 0x100;
        s.fpr[5] = 0x3FF8_0000_0000_0000;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Stfs {
                frs: 5,
                ra: 1,
                imm: 0,
            },
            &mut s,
            0,
            &[0u8; 0x200],
            &mut effects,
        );
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x100);
                assert_eq!(bytes.bytes(), &0x3FC0_0000u32.to_be_bytes());
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn lfd_preserves_nan_payload_8_bytes_verbatim() {
        // lfd is byte-for-byte; SNaN double pattern survives intact.
        let mut mem = vec![0u8; 0x100];
        let snan = 0x7FF0_0000_0000_0001u64;
        mem[0x10..0x18].copy_from_slice(&snan.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[1] = 0x10;
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lfd {
                frt: 5,
                ra: 1,
                imm: 0,
            },
            &mut s,
            0,
            &mem,
            &mut effects,
        );
        assert_eq!(s.fpr[5], snan);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "lfsu invalid form")]
    fn lfsu_with_ra_zero_panics_in_debug() {
        let mut s = PpuState::new();
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lfsu {
                frt: 5,
                ra: 0,
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
    #[should_panic(expected = "lfdu invalid form")]
    fn lfdu_with_ra_zero_panics_in_debug() {
        let mut s = PpuState::new();
        let mut effects = Vec::new();
        exec_with_mem(
            &PpuInstruction::Lfdu {
                frt: 5,
                ra: 0,
                imm: 0,
            },
            &mut s,
            0,
            &[0u8; 0x100],
            &mut effects,
        );
    }

    // Helper used by the lvsl test above.
    fn exec_no_mem_or_load<F>(s: &mut PpuState, e: &mut Vec<Effect>, f: F)
    where
        F: FnOnce(&mut PpuState, &mut Vec<Effect>) -> ExecuteVerdict,
    {
        let v = f(s, e);
        assert_eq!(v, ExecuteVerdict::Continue);
    }

    // ------------------------------------------------------------------
    // Reservation side-effect coverage (Lwarx / Ldarx / Stwcx. / Stdcx.)
    // ------------------------------------------------------------------

    #[test]
    fn lwarx_sets_reservation_on_aligned_ea() {
        let mut mem = vec![0u8; 0x2000];
        mem[0x1004..0x1008].copy_from_slice(&0xCAFE_BABEu32.to_be_bytes());
        let mut s = PpuState::new();
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x4;
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
    fn lwarx_misaligned_ea_raises_alignment_fault() {
        let mem = vec![0u8; 0x2000];
        let mut s = PpuState::new();
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x1;
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
        assert_eq!(
            result,
            ExecuteVerdict::Fault(PpuFault::AlignmentInterrupt(0x1001))
        );
        assert!(s.reservation.is_none());
        assert!(effects.is_empty());
    }

    #[test]
    fn ldarx_sets_reservation_on_8_byte_aligned_ea() {
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
    fn ldarx_misaligned_ea_raises_alignment_fault() {
        let mem = vec![0u8; 0x2000];
        let mut s = PpuState::new();
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x4;
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
        assert_eq!(
            result,
            ExecuteVerdict::Fault(PpuFault::AlignmentInterrupt(0x1004))
        );
        assert!(s.reservation.is_none());
        assert!(effects.is_empty());
    }

    #[test]
    fn stwcx_sets_cr0_eq_when_reservation_held() {
        let mut s = PpuState::new();
        s.reservation = Some(ReservedLine::containing(0x1000));
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0;
        s.gpr[5] = 0xAABB_CCDD;
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
        // LT=0, GT=0, EQ=1, SO=0.
        assert_eq!(s.cr_field(0), 0b0010);
    }

    #[test]
    fn stwcx_sets_cr0_neq_when_reservation_lost() {
        let mut s = PpuState::new();
        // No reservation held -> conditional store fails.
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0;
        s.gpr[5] = 0xAABB_CCDD;
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
        // LT=0, GT=0, EQ=0, SO=0.
        assert_eq!(s.cr_field(0), 0b0000);
    }

    #[test]
    fn stwcx_clears_reservation_regardless_of_success() {
        let mut s = PpuState::new();
        s.reservation = Some(ReservedLine::containing(0x1000));
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0;
        s.gpr[5] = 0;
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
        assert!(s.reservation.is_none());
    }

    #[test]
    fn stwcx_clears_reservation_on_failure_too() {
        let mut s = PpuState::new();
        // Reservation on a different line -> stwcx fails, but the
        // reservation must still be retired.
        s.reservation = Some(ReservedLine::containing(0x2000));
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0;
        s.gpr[5] = 0;
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
        assert!(s.reservation.is_none());
    }

    #[test]
    fn stwcx_propagates_xer_so_into_cr0() {
        // Failure path keeps the SO bit so CR0 = 0b0001.
        let mut s = PpuState::new();
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0;
        s.gpr[5] = 0;
        s.xer |= 1u64 << 31; // sticky SO directly.
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
        assert!(s.xer_so());
        assert_eq!(s.cr_field(0), 0b0001);
    }

    #[test]
    fn stwcx_always_clears_cr0_lt_and_gt() {
        // Pre-poison CR0 with LT=1, GT=1, EQ=0, SO=0. stwcx must
        // overwrite the whole field, not OR into it.
        let mut s = PpuState::new();
        s.set_cr_field(0, 0b1100);
        s.reservation = Some(ReservedLine::containing(0x1000));
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0;
        s.gpr[5] = 0;
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
        // Success: 0b0010. LT and GT must have been cleared.
        assert_eq!(s.cr_field(0), 0b0010);

        // Same check on the failure path.
        let mut s = PpuState::new();
        s.set_cr_field(0, 0b1100);
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0;
        s.gpr[5] = 0;
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
    }

    #[test]
    fn stwcx_misaligned_ea_raises_alignment_fault() {
        let mut s = PpuState::new();
        s.reservation = Some(ReservedLine::containing(0x1000));
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x2;
        s.gpr[5] = 0;
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
        assert_eq!(
            result,
            ExecuteVerdict::Fault(PpuFault::AlignmentInterrupt(0x1002))
        );
        // Fault path returns early: CR0 untouched, reservation intact,
        // no effects emitted.
        assert_eq!(s.cr_field(0), 0b0000);
        assert_eq!(s.reservation.map(|l| l.addr()), Some(0x1000));
        assert!(effects.is_empty());
    }

    #[test]
    fn stdcx_sets_cr0_eq_when_reservation_held() {
        let mut s = PpuState::new();
        s.reservation = Some(ReservedLine::containing(0x1000));
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x8;
        s.gpr[5] = 0xAABB_CCDD_EEFF_0011;
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
        assert_eq!(s.cr_field(0), 0b0010);
    }

    #[test]
    fn stdcx_sets_cr0_neq_when_reservation_lost() {
        let mut s = PpuState::new();
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x8;
        s.gpr[5] = 0;
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
    }

    #[test]
    fn stdcx_clears_reservation_regardless_of_success() {
        let mut s = PpuState::new();
        s.reservation = Some(ReservedLine::containing(0x1000));
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x8;
        s.gpr[5] = 0;
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
    fn stdcx_clears_reservation_on_failure_too() {
        let mut s = PpuState::new();
        s.reservation = Some(ReservedLine::containing(0x2000));
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x8;
        s.gpr[5] = 0;
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
        assert!(s.reservation.is_none());
    }

    #[test]
    fn stdcx_propagates_xer_so_into_cr0() {
        let mut s = PpuState::new();
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x8;
        s.gpr[5] = 0;
        s.xer |= 1u64 << 31;
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
        assert!(s.xer_so());
        assert_eq!(s.cr_field(0), 0b0001);
    }

    #[test]
    fn stdcx_always_clears_cr0_lt_and_gt() {
        let mut s = PpuState::new();
        s.set_cr_field(0, 0b1100);
        s.reservation = Some(ReservedLine::containing(0x1000));
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x8;
        s.gpr[5] = 0;
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
        assert_eq!(s.cr_field(0), 0b0010);

        let mut s = PpuState::new();
        s.set_cr_field(0, 0b1100);
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x8;
        s.gpr[5] = 0;
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
    }

    #[test]
    fn stdcx_misaligned_ea_raises_alignment_fault() {
        let mut s = PpuState::new();
        s.reservation = Some(ReservedLine::containing(0x1000));
        s.gpr[3] = 0x1000;
        s.gpr[4] = 0x4;
        s.gpr[5] = 0;
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
        assert_eq!(
            result,
            ExecuteVerdict::Fault(PpuFault::AlignmentInterrupt(0x1004))
        );
        assert_eq!(s.cr_field(0), 0b0000);
        assert_eq!(s.reservation.map(|l| l.addr()), Some(0x1000));
        assert!(effects.is_empty());
    }
}
