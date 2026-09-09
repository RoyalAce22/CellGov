//! The CLI's own progress vocabulary: the phases its long-running
//! commands move through, and the task descriptors a renderer lays them
//! out from.
//!
//! The installer's vocabulary is `cellgov_install::progress`; only the
//! labels are per-command, the sink and the renderer are shared.

use cellgov_terminal::progress::{Task, Unit};

pub(crate) use cellgov_terminal::progress::ProgressSink;

/// Steps a loop retires between two reports to its sink.
///
/// The render thread ticks at 10 Hz and a boot retires millions of
/// steps a second, so a batch this size is finer than a frame shows.
/// The loop then pays one predictable branch per step instead of an
/// atomic add.
pub(crate) const STEP_REPORT_BATCH: usize = 8192;

/// A coarse stage of a boot, indexing [`BENCH_TASK`]'s and
/// [`RUN_TASK`]'s label tables.
///
/// The pair ends at the step loop. A boot reports itself finished
/// there, and the witness and diagnostic block that follows is the
/// run's result rather than its progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum BootPhase {
    /// Reading the image, loading firmware, and running module_start.
    Loading = 0,
    /// The step loop; [`enter_step_loop`] sets its denominator.
    Stepping = 1,
}

impl BootPhase {
    /// The code a [`ProgressSink`] stores.
    pub(crate) const fn code(self) -> u8 {
        self as u8
    }
}

/// Declare `finish_line`, the step count the run should end at, as the
/// step phase's denominator, then enter the phase.
///
/// With no finish line the phase declares no totals: it counts the
/// steps it retires and predicts nothing.
/// [`crate::game::anchor_finish_line`] answers the finish line.
pub(crate) fn enter_step_loop(progress: &dyn ProgressSink, finish_line: Option<u64>) {
    if let Some(steps) = finish_line {
        progress.totals(0, steps);
    }
    progress.phase(BootPhase::Stepping.code());
}

/// How a renderer presents one bench measurement.
///
/// A bench boot is not quiet while it works: the PRX loader and every
/// firmware module's `module_start` write to stdout through the load
/// phase, and a child's init pass writes from inside the step loop.
pub(crate) const BENCH_TASK: Task = Task {
    verb: "Booting",
    tag: "bench",
    phases: &["loading", "stepping"],
    measured: BootPhase::Stepping as u8,
    unit: Unit::Steps,
    items: "",
    streaming: true,
};

/// How a renderer presents `boot run`.
///
/// Guest TTY captures and `--trace` lines stream to stdout throughout
/// the step loop, so this task can never own the terminal.
pub(crate) const RUN_TASK: Task = Task {
    verb: "Booting",
    tag: "boot",
    phases: &["loading", "stepping"],
    measured: BootPhase::Stepping as u8,
    unit: Unit::Steps,
    items: "",
    streaming: true,
};

/// A stage of the gating pair, indexing [`BENCH_PAIR_TASK`]'s labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum BenchPairPhase {
    /// Both measurements, each in its own subprocess.
    Measuring = 0,
    /// The determinism, anchor and wall-drift gates.
    Comparing = 1,
}

impl BenchPairPhase {
    /// The code a [`ProgressSink`] stores.
    pub(crate) const fn code(self) -> u8 {
        self as u8
    }
}

/// How a renderer presents `boot bench`.
///
/// The pair measures in subprocesses whose output it captures, so the
/// denominator is the two runs rather than their steps. Each run's
/// result line prints as soon as that run returns, which is why this
/// task streams.
pub(crate) const BENCH_PAIR_TASK: Task = Task {
    verb: "Benchmarking",
    tag: "bench",
    phases: &["measuring", "comparing"],
    measured: BenchPairPhase::Measuring as u8,
    unit: Unit::Items,
    items: "",
    streaming: true,
};

/// How a renderer presents `dev record-anchors`.
///
/// One item per title. Each title's verdict line prints as that title
/// lands, which is why this task streams.
pub(crate) const RECORD_ANCHORS_TASK: Task = Task {
    verb: "Recording",
    tag: "anchors",
    phases: &["measuring"],
    measured: 0,
    unit: Unit::Items,
    items: "",
    streaming: true,
};

#[cfg(test)]
#[path = "tests/progress_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/step_loop_entry_tests.rs"]
mod step_loop_entry_tests;
