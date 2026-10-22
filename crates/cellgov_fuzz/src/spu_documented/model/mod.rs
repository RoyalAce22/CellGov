//! What the documents say each floating-point instruction does: the
//! register pieces it writes and the FPSCR flags it raises, each either
//! fixed by a page or left open with a named question.

mod double;
mod effect;
mod single;

pub(crate) use double::double_operand;
#[cfg(test)]
pub(crate) use double::{double_slot, round_to_single};
pub(crate) use effect::{effect, fpscr_bit, with_mode, Insn, Known, Op};
pub(crate) use single::{extended_operand, extended_result};
#[cfg(test)]
pub(crate) use single::{interpolate, to_integer};
