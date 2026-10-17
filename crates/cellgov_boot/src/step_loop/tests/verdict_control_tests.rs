//! A reserved write to the RSX context window's control block is the
//! `FirstRsxWrite` checkpoint, as one to the RSX region is.

use super::*;
use cellgov_mem::MemError;
use cellgov_ps3_abi::lv2::rsx::control_register::PUT_ADDR;
use manifest::CheckpointTrigger;

fn control_write() -> CommitError {
    CommitError::Memory(MemError::ReservedWrite {
        addr: u64::from(PUT_ADDR),
        region: crate::prepare::RSX_CONTROL_REGION,
    })
}

#[test]
fn a_reserved_put_write_in_the_control_block_is_the_checkpoint() {
    assert_eq!(
        rsx_checkpoint_addr(CheckpointTrigger::FirstRsxWrite, control_write()),
        Some(u64::from(PUT_ADDR))
    );
}

#[test]
fn a_control_block_write_is_no_checkpoint_under_another_trigger() {
    assert_eq!(
        rsx_checkpoint_addr(CheckpointTrigger::ProcessExit, control_write()),
        None
    );
}
