use super::*;
use crate::manifest::CheckpointTrigger;

#[test]
fn a_run_ends_at_the_checkpoint_it_was_given() {
    for cp in [
        CheckpointTrigger::ProcessExit,
        CheckpointTrigger::FirstRsxWrite,
    ] {
        assert!(step_loop_ends_at(cp, cp), "{}", cp.as_cli_str());
    }
    assert!(!step_loop_ends_at(
        CheckpointTrigger::ProcessExit,
        CheckpointTrigger::FirstRsxWrite
    ));
}

#[test]
fn a_pc_checkpoint_is_not_where_the_run_ends() {
    let pc = CheckpointTrigger::Pc(0x1_0000);
    assert!(!step_loop_ends_at(pc, pc));
}
