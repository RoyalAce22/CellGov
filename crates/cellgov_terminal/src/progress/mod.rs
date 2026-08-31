//! Progress reporting: the trait instrumented code emits against, and
//! the terminal renderer that draws it.
//!
//! A caller describes its work once as a [`Task`] -- verb, phase
//! labels, denominator unit -- and hands it to [`ProgressBar::start`];
//! the renderer composes each frame from that descriptor plus the
//! counters the [`ProgressSink`] accumulates.

mod bar;
mod frame;
mod sink;
mod state;
mod task;

pub use bar::ProgressBar;
pub use sink::ProgressSink;
pub use state::ProgressState;
pub use task::{Task, Unit};
