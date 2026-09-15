//! Step drivers for `boot run` (diagnostic) and `boot bench`
//! (throughput).
//!
//! Both loops share `verdict::classify_step_outcome` for verdict
//! precedence.

mod bench;
mod block_reason;
mod ctx;
mod driver;
mod ring;
mod timing;
mod verdict;

pub(crate) mod tty;

pub(crate) use block_reason::block_reason_label;

/// Steps a loop retires between two reports to its progress sink.
///
/// The render thread ticks at 10 Hz and a boot retires millions of
/// steps a second, so a batch this size is finer than a frame shows.
/// The loop then pays one predictable branch per step instead of an
/// atomic add.
pub const STEP_REPORT_BATCH: usize = 8192;

pub use bench::bench_step_loop;
pub use ctx::StepLoopCtx;
pub use driver::step_loop;
pub use ring::{PcRing, Ring, SyscallRing};
pub use timing::{compute_untracked, pct, StepTiming};
