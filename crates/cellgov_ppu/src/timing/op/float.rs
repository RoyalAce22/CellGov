//! The builders for floating-point instructions.

use super::model::{crf, fpr, unit, Kind, LoadClass, Op, FPSCR};
use super::storage::{load, VSU_MEM};
use crate::instruction::ops::{Fp59Op, Fp63Op};
use cellgov_ps3_abi::hw::ppu_issue as k;

fn fp(frt: u8, srcs: &[u8], lat: u64, rc: bool) -> Op {
    let mut op = Op::on(unit::VSU1).dst(fpr(frt), lat);
    for &s in srcs {
        op = op.src(fpr(s));
    }
    if rc {
        op = op.dst(crf(1), lat + k::PPU_FP_RECORD_EXTRA_CYCLES);
    }
    op
}

fn fp_unpipelined(frt: u8, srcs: &[u8], lat: u64, rc: bool) -> Op {
    let mut op = fp(frt, srcs, lat, rc).kind(Kind::Unpipelined(lat as u16));
    op.units = unit::VSU1 | unit::VSU2;
    op
}

pub(super) fn fp_load(ea: u64, len: u8, frt: u8) -> Op {
    load(VSU_MEM, ea, len, LoadClass::Float).dst(fpr(frt), k::PPU_FP_LOAD_LATENCY_CYCLES)
}

pub(super) fn fp63(op: Fp63Op, frt: u8, fra: u8, frb: u8, frc: u8, rc: bool) -> Op {
    use Fp63Op as F;
    let l = k::PPU_FP_LATENCY_CYCLES;
    match op {
        F::Fadd | F::Fsub => fp(frt, &[fra, frb], l, rc),
        F::Fmul => fp(frt, &[fra, frc], l, rc),
        F::Fmadd | F::Fmsub | F::Fnmadd | F::Fnmsub | F::Fsel => fp(frt, &[fra, frc, frb], l, rc),
        F::Frsp
        | F::Fctiw
        | F::Fctiwz
        | F::Fctid
        | F::Fctidz
        | F::Fcfid
        | F::Fmr
        | F::Fneg
        | F::Fabs
        | F::Fnabs
        | F::Frsqrte => fp(frt, &[frb], l, rc),
        F::Fdiv => fp_unpipelined(frt, &[fra, frb], k::PPU_FDIV_CYCLES, rc),
        F::Fsqrt => fp_unpipelined(frt, &[frb], k::PPU_FSQRT_CYCLES, rc),
        // [CBE-Handbook p:722 s:A.1.2] `fcmpu fcmpo`: latency 1.
        F::Fcmpu | F::Fcmpo => Op::on(unit::VSU1)
            .src(fpr(fra))
            .src(fpr(frb))
            .dst(crf(frt >> 2), k::PPU_COMPARE_LATENCY_CYCLES)
            .dst(FPSCR, k::PPU_COMPARE_LATENCY_CYCLES),
        F::Mffs => Op::on(unit::VSU1)
            .src(FPSCR)
            .dst(fpr(frt), k::PPU_MFFS_LATENCY_CYCLES)
            .kind(Kind::Drain),
        // [CBE-Handbook p:722 s:A.1.2] `mtfsfi mtfsf mtfsb0 mtfsb1`:
        // latency 1.
        F::Mtfsb0 | F::Mtfsb1 | F::Mtfsfi => {
            Op::on(unit::VSU1).dst(FPSCR, k::PPU_COMPARE_LATENCY_CYCLES)
        }
        F::Mtfsf => Op::on(unit::VSU1)
            .src(fpr(frb))
            .dst(FPSCR, k::PPU_COMPARE_LATENCY_CYCLES),
        // [CBE-Handbook p:729 s:A.1.3.1] `mcrfs`: 2 microwords.
        F::Mcrfs => Op::on(unit::VSU1)
            .src(FPSCR)
            .dst(crf(frt >> 2), 2)
            .microcode(2, 2),
    }
}

pub(super) fn fp59(op: Fp59Op, frt: u8, fra: u8, frb: u8, frc: u8, rc: bool) -> Op {
    use Fp59Op as F;
    let l = k::PPU_FP_LATENCY_CYCLES;
    match op {
        F::Fadds | F::Fsubs => fp(frt, &[fra, frb], l, rc),
        F::Fmuls => fp(frt, &[fra, frc], l, rc),
        F::Fmadds | F::Fmsubs | F::Fnmadds | F::Fnmsubs => fp(frt, &[fra, frc, frb], l, rc),
        F::Fres => fp(frt, &[frb], l, rc),
        F::Fdivs => fp_unpipelined(frt, &[fra, frb], k::PPU_FDIV_CYCLES, rc),
        F::Fsqrts => fp_unpipelined(frt, &[frb], k::PPU_FSQRT_CYCLES, rc),
    }
}
