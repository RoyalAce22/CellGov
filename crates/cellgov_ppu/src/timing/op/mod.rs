//! What the issue model needs to know of one instruction: its units, the
//! registers it reads and writes, and the latency of each write.
//!
//! Latencies follow Table A-1 and Table A-8 of the handbook, with the
//! console's measured values where the capture times a term.
//! [CBE-Handbook p:715 s:A.1.2] the latency of an instruction is its
//! issue-to-issue latency to a dependent instruction.

mod classify;
mod fixed;
mod float;
mod fused;
mod model;
mod storage;
mod vector;

pub(crate) use classify::classify;
#[cfg(test)]
pub(super) use model::XER;
pub(super) use model::{is_fpr, is_vr};
pub(crate) use model::{unit, Barrier, Kind, LoadClass, Op, Target, NO_REG, REGS};
