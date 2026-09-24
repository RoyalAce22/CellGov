//! How `explore title` words a window that never opened.
//!
//! [`cellgov_explore::open_window`] drives the boot to the window's
//! start; this module names what stopped it in the cell's terms, since
//! only the cell's checkpoint separates the boot's own stop from a
//! refusal.

use cellgov_boot::manifest::CheckpointTrigger;
use cellgov_boot::step_loop::rsx_checkpoint_addr;
use cellgov_effects::FaultKind;
use cellgov_explore::{DrivenStop, StopReason, WindowNeverOpened, WindowStart};

/// The refusal line for a window that never opened, under the cell
/// whose anchor stops the boot at `checkpoint`.
///
/// A `first-rsx-write` checkpoint arrives as a refused commit; see
/// [`rsx_checkpoint_addr`].
pub(super) fn never_opened(e: &WindowNeverOpened, checkpoint: CheckpointTrigger) -> String {
    format!(
        "explore title: the window never opened at {}: {}",
        e.start,
        detail(e, checkpoint)
    )
}

fn detail(e: &WindowNeverOpened, checkpoint: CheckpointTrigger) -> String {
    if let StopReason::CommitError(err) = e.stop.reason {
        if let Some(addr) = rsx_checkpoint_addr(checkpoint, err) {
            return format!(
                "after {} step(s) the boot reached the cell's {} checkpoint at \
                 0x{addr:08x}, which is where it stops. Open the window earlier.",
                e.steps,
                checkpoint.as_cli_str(),
            );
        }
    }
    format!(
        "the boot stopped after {} step(s) -- {} ({})",
        e.steps,
        stop_clause(e.stop),
        e.stop.reason.class().label(),
    )
}

/// What stopped the boot, in the terms of a boot rather than a search.
fn stop_clause(stop: DrivenStop) -> String {
    match stop.reason {
        StopReason::Faulted(kind) => {
            let at = stop
                .pc
                .map(|p| format!("0x{p:08x}"))
                .unwrap_or_else(|| "<unknown>".to_string());
            match kind {
                FaultKind::Guest(code) => {
                    format!("a unit faulted at pc {at} with guest code 0x{code:08x}")
                }
                FaultKind::Validation => format!("a unit faulted at pc {at} on commit validation"),
            }
        }
        StopReason::ChildInitUnserved => "the boot spawned a child and staged its init pass, \
             which this command does not run; the child's primary thread would stay parked \
             and the window would cover a boot the cell's anchor does not describe"
            .to_string(),
        reason => reason.to_string(),
    }
}

/// Why `start` cannot open inside a runtime whose step cap is
/// `max_steps`, or `None` when it can.
///
/// A window with no step in it would read as a stable one. `--max-steps`
/// is a retired-instruction cap; the boot divides it by the per-step
/// budget into the runtime step cap this compares against.
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
