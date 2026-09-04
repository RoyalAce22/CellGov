//! Step drivers for `boot run` (diagnostic) and `boot bench` (throughput).
//!
//! Both loops share [`verdict::classify_step_outcome`] for verdict
//! precedence.

mod bench;
mod block_reason;
mod ctx;
mod driver;
mod ring;
mod timing;
pub(super) mod tty;
mod verdict;

pub(super) use bench::bench_step_loop;
pub(super) use block_reason::block_reason_label;
pub(super) use ctx::StepLoopCtx;
pub(super) use driver::step_loop;
pub(super) use ring::{PcRing, SyscallRing};
pub(super) use timing::{compute_untracked, pct, StepTiming};
