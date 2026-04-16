//! Floating-point instruction execution helpers.
//!
//! Dispatches opcode-63 (double precision) and opcode-59 (single
//! precision) A-form and X-form FP instructions. Called from
//! `exec::execute` for `Fp63` and `Fp59` instruction variants.

use crate::exec::ExecuteVerdict;
use crate::state::PpuState;

pub fn execute_fp63(
    state: &mut PpuState,
    xo: u16,
    frt: u8,
    fra: u8,
    frb: u8,
    frc: u8,
) -> ExecuteVerdict {
    let a = f64::from_bits(state.fpr[fra as usize]);
    let b = f64::from_bits(state.fpr[frb as usize]);
    let c = f64::from_bits(state.fpr[frc as usize]);
    let xo5 = xo & 0x1F;
    let result_bits = match xo5 {
        29 => (a * c + b).to_bits(),
        28 => (a * c - b).to_bits(),
        31 => (-(a * c + b)).to_bits(),
        30 => (-(a * c - b)).to_bits(),
        25 => (a * c).to_bits(),
        23 => {
            if a >= 0.0 {
                c.to_bits()
            } else {
                b.to_bits()
            }
        }
        _ => match xo {
            18 => (a / b).to_bits(),
            21 => (a + b).to_bits(),
            20 => (a - b).to_bits(),
            72 => b.to_bits(),
            40 => (-b).to_bits(),
            264 => b.abs().to_bits(),
            136 => (-b.abs()).to_bits(),
            0 | 32 => {
                let bf = (frt >> 2) & 7;
                let cr_val = if a.is_nan() || b.is_nan() {
                    0b0001
                } else if a < b {
                    0b1000
                } else if a > b {
                    0b0100
                } else {
                    0b0010
                };
                state.set_cr_field(bf, cr_val);
                return ExecuteVerdict::Continue;
            }
            12 => {
                let s = b as f32;
                (s as f64).to_bits()
            }
            15 => {
                let i = b as i32;
                (i as u64) & 0xFFFFFFFF
            }
            814 | 815 => {
                let i = b as i64;
                i as u64
            }
            846 => {
                let i = state.fpr[frb as usize] as i64;
                (i as f64).to_bits()
            }
            _ => return ExecuteVerdict::Continue,
        },
    };
    state.fpr[frt as usize] = result_bits;
    ExecuteVerdict::Continue
}

pub fn execute_fp59(
    state: &mut PpuState,
    xo: u16,
    frt: u8,
    fra: u8,
    frb: u8,
    frc: u8,
) -> ExecuteVerdict {
    let a = f64::from_bits(state.fpr[fra as usize]) as f32;
    let b = f64::from_bits(state.fpr[frb as usize]) as f32;
    let c = f64::from_bits(state.fpr[frc as usize]) as f32;
    let xo5 = xo & 0x1F;
    let result = match xo5 {
        29 => a * c + b,
        28 => a * c - b,
        31 => -(a * c + b),
        30 => -(a * c - b),
        25 => a * c,
        _ => match xo {
            18 => a / b,
            21 => a + b,
            20 => a - b,
            _ => return ExecuteVerdict::Continue,
        },
    };
    state.fpr[frt as usize] = (result as f64).to_bits();
    ExecuteVerdict::Continue
}
