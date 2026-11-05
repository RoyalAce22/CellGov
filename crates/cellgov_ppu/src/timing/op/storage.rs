//! The builders for loads, stores, the multiples and the strings.

use super::model::{gpr, unit, Kind, LoadClass, Op, Reg};
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu_issue as k;

pub(super) fn fx(rt: Reg) -> Op {
    Op::on(unit::FXU).dst(rt, k::PPU_FX_LATENCY_CYCLES)
}

pub(super) fn ea_d(state: &PpuState, ra: u8, imm: i16) -> u64 {
    let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
    base.wrapping_add(imm as i64 as u64)
}

pub(super) fn ea_x(state: &PpuState, ra: u8, rb: u8) -> u64 {
    let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
    base.wrapping_add(state.gpr[rb as usize])
}

pub(super) fn load(units: u8, ea: u64, len: u8, class: LoadClass) -> Op {
    Op::on(units).kind(Kind::Load { ea, len, class })
}

pub(super) fn store(units: u8, ea: u64, len: u8) -> Op {
    Op::on(units).kind(Kind::Store {
        ea,
        len,
        conditional: false,
    })
}

/// A fixed-point load's write of `rt`: the L1 latency to a dependent
/// fixed-point operation. The pipeline replaces it on a miss.
pub(super) fn int_load(ea: u64, len: u8, rt: u8) -> Op {
    load(unit::LSU, ea, len, LoadClass::Fixed).dst(gpr(rt), k::PPU_LOAD_FX_USE_CYCLES)
}

/// A load or store with update also writes RA through the FXU.
///
/// [CBE-Handbook p:716 s:A.1.2] load and store with update: the
/// hardware breaks it into the basic load or store and an add.
pub(super) fn update(op: Op, ra: u8) -> Op {
    let mut op = op.dst(gpr(ra), k::PPU_FX_LATENCY_CYCLES);
    op.units |= unit::FXU;
    op
}

pub(super) const INT_STORE: u8 = unit::FXU | unit::LSU;
pub(super) const VSU_MEM: u8 = unit::LSU | unit::VSU2;

/// A load algebraic, which the microcode handles by byte.
///
/// `lha` takes the console's microword count; the other forms take the
/// handbook's.
///
/// [CBE-Handbook p:729 s:A.1.3.1] `lha` 7 microwords, `lhau lhaux lhax`
/// 8, `lwa` 13, `lwaux lwax` 12; latency 11.
pub(super) fn load_algebraic(ea: u64, len: u8, rt: u8, words: u8) -> Op {
    load(unit::LSU | unit::FXU, ea, len, LoadClass::Fixed)
        .dst(gpr(rt), words as u64 + k::PPU_LOAD_FX_USE_CYCLES)
        .microcode(words, words)
}

/// A load or store multiple, or a load or store string, of `bytes` bytes
/// from register `first`.
///
/// [CBE-Handbook p:729 s:A.1.3.1] `lmw stmw`: 2 + 1 x words microwords;
/// `lswi stswi`: 1 x words + 2 x bytes.
pub(super) fn multiple(ea: u64, first: u8, bytes: u64, store_side: bool, string: bool) -> Op {
    let words = bytes / 4;
    let tail = bytes % 4;
    let micro = if string { words + 2 * tail } else { 2 + words };
    let micro = micro.clamp(1, u8::MAX as u64) as u8;
    let regs = bytes.div_ceil(4).min(32) as u8;
    let len = bytes.min(u8::MAX as u64) as u8;
    if store_side {
        let span = if string {
            micro
        } else {
            micro.saturating_add(k::PPU_STORE_MULTIPLE_EXTRA_CYCLES)
        };
        let mut op = store(INT_STORE, ea, len).microcode(micro, span);
        op.src_range = Some((gpr(first), regs, k::PPU_STORE_SOURCE_LEAD_MICROWORDS));
        op
    } else {
        let mut op = load(unit::LSU | unit::FXU, ea, len, LoadClass::Fixed).microcode(micro, micro);
        op.range = Some((
            gpr(first),
            regs,
            (micro as u64 + k::PPU_LOAD_FX_USE_CYCLES) as u16,
        ));
        op
    }
}
