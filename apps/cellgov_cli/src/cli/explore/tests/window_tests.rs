//! How `explore title` words a window that never opened.

use super::*;
use cellgov_core::StepError;
use cellgov_mem::MemError;

/// The checkpoint every case below runs under; the RSX cases name
/// their own.
const EXITS: CheckpointTrigger = CheckpointTrigger::ProcessExit;

fn ended(
    start: WindowStart,
    steps: usize,
    reason: StopReason,
    pc: Option<u64>,
) -> WindowNeverOpened {
    WindowNeverOpened {
        start,
        steps,
        stop: DrivenStop { reason, pc },
    }
}

#[test]
fn a_cap_is_named_a_bound_with_the_start_it_never_reached() {
    let text = never_opened(
        &ended(
            WindowStart::Pc(0xDEAD_BEEF),
            2,
            StopReason::StepError(StepError::MaxStepsExceeded),
            None,
        ),
        EXITS,
    );
    assert!(text.contains("pc 0xdeadbeef"), "{text}");
    assert!(text.contains("after 2 step(s)"), "{text}");
    assert!(
        text.contains("(bound)"),
        "the cap is the caller's own and must not read as a model refusal: {text}"
    );
}

#[test]
fn a_fault_names_the_pc_and_the_guest_code() {
    let text = never_opened(
        &ended(
            WindowStart::FirstBranchingPoint,
            0,
            StopReason::Faulted(FaultKind::Guest(0x0000_0700)),
            Some(0x1_0040),
        ),
        EXITS,
    );
    assert!(text.contains("0x00010040"), "{text}");
    assert!(text.contains("0x00000700"), "{text}");
    assert!(text.contains("(fault)"), "{text}");
}

#[test]
fn a_parked_boot_reads_as_a_deadlock() {
    let text = never_opened(
        &ended(
            WindowStart::FirstBranchingPoint,
            0,
            StopReason::Deadlocked,
            None,
        ),
        EXITS,
    );
    assert!(text.contains("deadlocked"), "{text}");
    assert!(text.contains("(blocked)"), "{text}");
}

#[test]
fn a_start_step_at_or_past_the_step_cap_is_refused_before_the_boot_runs() {
    // The refusal has to name both numbers: an operator who reads only
    // "too large" cannot tell which of the two flags to move.
    let at = start_past_cap(WindowStart::Step(100), 100).expect("the cap is not past itself");
    assert!(at.contains("100"), "{at}");
    assert!(at.contains("--start-step"), "{at}");
    assert!(at.contains("--max-steps"), "{at}");

    let past = start_past_cap(WindowStart::Step(4_096), 100).expect("4096 is past the cap");
    assert!(past.contains("4096"), "{past}");
    assert!(past.contains("100"), "{past}");

    assert!(start_past_cap(WindowStart::Step(99), 100).is_none());
    assert!(
        start_past_cap(WindowStart::FirstBranchingPoint, 1).is_none(),
        "only a step count can be placed past the cap",
    );
    assert!(start_past_cap(WindowStart::Pc(0x1000), 1).is_none());
}

#[test]
fn a_boot_that_reached_the_cell_s_checkpoint_is_not_reported_as_a_refusal() {
    let rsx_write =
        StopReason::CommitError(cellgov_core::CommitError::Memory(MemError::ReservedWrite {
            addr: 0x0C00_0000,
            region: "rsx",
        }));
    let at_checkpoint = never_opened(
        &ended(WindowStart::Step(1_000), 900, rsx_write, None),
        CheckpointTrigger::FirstRsxWrite,
    );
    assert!(at_checkpoint.contains("first-rsx-write"), "{at_checkpoint}");
    assert!(at_checkpoint.contains("0x0c000000"), "{at_checkpoint}");
    assert!(
        !at_checkpoint.contains("(refusal)"),
        "the cell's own stop is not a refusal the model gave: {at_checkpoint}",
    );

    // The same refusal in a cell that stops elsewhere keeps its class.
    let elsewhere = never_opened(
        &ended(WindowStart::Step(1_000), 900, rsx_write, None),
        EXITS,
    );
    assert!(elsewhere.contains("(refusal)"), "{elsewhere}");
}

/// A real `sys_process_spawn` drives the trigger, and only the
/// `microtests` suite reaches one; this case pins the line an operator
/// reads when it fires.
#[test]
fn an_unserved_child_init_is_reported_as_neither_a_stall_nor_a_refusal() {
    let text = never_opened(
        &ended(
            WindowStart::FirstBranchingPoint,
            412,
            StopReason::ChildInitUnserved,
            None,
        ),
        EXITS,
    );
    assert!(text.contains("412"), "{text}");
    assert!(text.contains("spawned a child"), "{text}");
    assert!(text.contains("(unserved)"), "{text}");
    assert!(
        !text.contains("(finished)") && !text.contains("(refusal)"),
        "a parked child is neither a boot that ran itself out nor a refusal: {text}",
    );
}
