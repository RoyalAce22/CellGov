//! The PPE's in-order issue model: the core cycles a PPU thread's
//! instructions take.
//!
//! [`op`] classifies each retired instruction: the units it occupies, the
//! registers it reads and writes, and the latency of each write.
//! [`pipeline`] then issues it against the fetch, dispatch and issue
//! rules. Loads look up the tag-only caches of [`cache`]. Branches use
//! the branch history table and the link stack of [`predictor`]. A
//! thread's cycle is the earliest cycle its next instruction can issue
//! at. A batch charges the cycles its instructions advance that cycle.

mod cache;
mod op;
mod pipeline;
mod predictor;

pub(crate) use op::{classify, Op};
pub(crate) use pipeline::{Core, PpeTiming};

#[cfg(test)]
#[path = "tests/timing_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/pressure_tests.rs"]
mod pressure_tests;
