//! Where a title exploration's window opens, and how the driver gets
//! the runtime there. A boot prefix is single-unit until the title
//! creates its second thread, so the driver runs the default schedule
//! to a start condition the caller picks and hands what follows to the
//! explorer.

use cellgov_boot::manifest::CheckpointTrigger;
use cellgov_boot::step_loop::rsx_checkpoint_addr;
use cellgov_core::{Runtime, StepError};
use cellgov_effects::FaultKind;
use cellgov_explore::StopReason;

/// Where the window opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WindowStart {
    /// The first step two or more units are runnable at.
    FirstBranchingPoint,
    /// A `Runtime::step()` count. `--max-steps` is a retired-instruction
    /// cap; the boot divides it by the per-step budget into the step cap
    /// this count runs against.
    Step(usize),
    /// A guest PC, matched against the PC a step yields at; a PC inside
    /// a batch never matches.
    Pc(u64),
}

impl std::fmt::Display for WindowStart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FirstBranchingPoint => f.write_str("first branching point"),
            Self::Step(n) => write!(f, "step {n}"),
            Self::Pc(addr) => write!(f, "pc 0x{addr:x}"),
        }
    }
}

/// What ended the drive to the window's start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WindowStop {
    /// A reason the explorer's own driver reports as well.
    Run(StopReason),
    /// A unit yielded on a fault, so the commit discarded its batch.
    Fault {
        /// The PC the unit yielded at, absent when it reported none.
        pc: Option<u64>,
        /// What the unit faulted with.
        kind: FaultKind,
    },
    /// The boot spawned a child whose staged init pass this drive does
    /// not run.
    ChildInitUnserved,
}

impl WindowStop {
    /// The one-word class the refusal line prints beside the reason.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Run(stop) => stop.class().label(),
            Self::Fault { .. } => "fault",
            Self::ChildInitUnserved => "unserved",
        }
    }
}

impl std::fmt::Display for WindowStop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Run(stop) => write!(f, "{stop}"),
            Self::Fault { pc, kind } => {
                let at = pc
                    .map(|p| format!("0x{p:08x}"))
                    .unwrap_or_else(|| "<unknown>".to_string());
                match kind {
                    FaultKind::Guest(code) => {
                        write!(f, "a unit faulted at pc {at} with guest code 0x{code:08x}")
                    }
                    FaultKind::Validation => {
                        write!(f, "a unit faulted at pc {at} on commit validation")
                    }
                }
            }
            Self::ChildInitUnserved => f.write_str(
                "the boot spawned a child and staged its init pass, which this command \
                 does not run; the child's primary thread would stay parked and the \
                 window would cover a boot the cell's anchor does not describe",
            ),
        }
    }
}

/// Why the boot ended before it met the start condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("explore title: the window never opened at {start}: {}", self.detail())]
pub(super) struct WindowNeverOpened {
    /// The condition the drive was running to.
    pub start: WindowStart,
    /// Where the cell's anchor stops the boot.
    pub checkpoint: CheckpointTrigger,
    /// Steps the boot committed before it stopped.
    pub steps: usize,
    /// What stopped it.
    pub stop: WindowStop,
}

impl WindowNeverOpened {
    /// What stopped the boot, as the clause the message ends with.
    ///
    /// A `first-rsx-write` checkpoint arrives as a refused commit; see
    /// [`rsx_checkpoint_addr`].
    fn detail(&self) -> String {
        if let WindowStop::Run(StopReason::CommitError(err)) = self.stop {
            if let Some(addr) = rsx_checkpoint_addr(self.checkpoint, err) {
                return format!(
                    "after {} step(s) the boot reached the cell's {} checkpoint at \
                     0x{addr:08x}, which is where it stops. Open the window earlier.",
                    self.steps,
                    self.checkpoint.as_cli_str(),
                );
            }
        }
        format!(
            "the boot stopped after {} step(s) -- {} ({})",
            self.steps,
            self.stop,
            self.stop.label(),
        )
    }
}

/// Run `rt` on its default schedule until it meets `start`.
///
/// Returns the `Runtime::step()` count the window opens at. This leaves
/// the runtime one `step()` from the window's first step, so the
/// explorer's baseline continues this same boot.
///
/// # Errors
///
/// [`WindowNeverOpened`] when the boot ends before the condition holds:
///
/// - it stalls or parks
/// - it hits a cap
/// - a unit faults
/// - the commit pipeline refuses a batch
/// - a child parks behind a staged init pass
///
/// The window then covers nothing, and the run has no verdict.
pub(super) fn open_window(
    rt: &mut Runtime,
    start: WindowStart,
    checkpoint: CheckpointTrigger,
) -> Result<usize, WindowNeverOpened> {
    let ended = |steps, stop| WindowNeverOpened {
        start,
        checkpoint,
        steps,
        stop,
    };
    loop {
        let runnable = rt.registry().runnable_ids().count();
        let steps = rt.steps_taken();
        match start {
            WindowStart::FirstBranchingPoint if runnable >= 2 => return Ok(steps),
            WindowStart::Step(n) if steps >= n => return Ok(steps),
            _ => {}
        }
        // An empty runnable set goes to `Runtime::step`, whose warp
        // alone separates `NoRunnableUnit` from `AllBlocked`.
        let mut step = match rt.step() {
            Ok(step) => step,
            Err(StepError::NoRunnableUnit) => {
                return Err(ended(steps, WindowStop::Run(StopReason::Stalled)))
            }
            Err(e) => return Err(ended(steps, WindowStop::Run(StopReason::StepError(e)))),
        };
        if let Err(e) = rt.commit_step_and_recycle(&mut step) {
            return Err(ended(steps, WindowStop::Run(StopReason::CommitError(e))));
        }
        // `boot run` drains this between steps; nothing here does. See
        // the doc on `WindowStop::ChildInitUnserved`.
        if rt.has_pending_child_init() {
            return Err(ended(steps, WindowStop::ChildInitUnserved));
        }
        // The precedence the boot's own step loop classifies a step by:
        // a discarded batch ends the boot.
        if let Some(kind) = step.result.fault {
            return Err(ended(
                steps,
                WindowStop::Fault {
                    pc: step.result.local_diagnostics.pc,
                    kind,
                },
            ));
        }
        if let (WindowStart::Pc(target), Some(pc)) = (start, step.result.local_diagnostics.pc) {
            if pc == target {
                return Ok(rt.steps_taken());
            }
        }
    }
}

/// Why `start` cannot open inside a runtime whose step cap is
/// `max_steps`, or `None` when it can.
///
/// A window with no step in it would read as a stable one.
pub(super) fn start_past_cap(start: WindowStart, max_steps: usize) -> Option<String> {
    let WindowStart::Step(n) = start else {
        return None;
    };
    (n >= max_steps).then(|| {
        format!(
            "explore title: --start-step {n} is at or past the boot's step cap \
             ({max_steps}), so the window would hold no step. Raise --max-steps, or open \
             the window earlier."
        )
    })
}

#[cfg(test)]
#[path = "tests/window_tests.rs"]
mod tests;
