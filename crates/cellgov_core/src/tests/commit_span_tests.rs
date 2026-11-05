//! A shared write whose range runs across two regions that sit end to
//! end, as a store runs across two pages mapped side by side.

use super::*;
use crate::commit::tests::{range, step_with, CommitTestBed};
use cellgov_effects::WritePayload;
use cellgov_event::UnitId;
use cellgov_mem::{GuestMemory, PageSize, Region};
use cellgov_time::GuestTicks;

const EDGE: u64 = 0x100;

fn write(start: u64, bytes: Vec<u8>) -> Effect {
    Effect::shared_write(
        range(start, bytes.len() as u64),
        WritePayload::new(bytes),
        UnitId::new(0xD0),
        GuestTicks::new(0),
    )
}

#[test]
fn a_shared_write_across_two_adjacent_regions_commits_into_both() {
    let mem = GuestMemory::from_regions(vec![
        Region::new(0, EDGE as usize, "low", PageSize::Page4K),
        Region::new(EDGE, 0x100, "high", PageSize::Page4K),
    ])
    .unwrap();
    let mut bed = CommitTestBed::with_memory(mem);
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![write(EDGE - 2, vec![1, 2, 3, 4])],
    );
    let outcome = bed.process(&r, &e).expect("every byte lies in a region");
    assert_eq!(outcome.writes_committed, 1);
    assert_eq!(
        bed.memory().read(range(EDGE - 2, 2)).as_deref().unwrap(),
        &[1, 2]
    );
    assert_eq!(
        bed.memory().read(range(EDGE, 2)).as_deref().unwrap(),
        &[3, 4]
    );
}

#[test]
fn a_shared_write_running_past_two_regions_into_a_gap_aborts_the_batch() {
    let mem = GuestMemory::from_regions(vec![
        Region::new(0, EDGE as usize, "low", PageSize::Page4K),
        Region::new(EDGE, 0x100, "high", PageSize::Page4K),
    ])
    .unwrap();
    let mut bed = CommitTestBed::with_memory(mem);
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![write(0, vec![9; 4]), write(2 * EDGE - 2, vec![1, 2, 3, 4])],
    );
    let err = bed
        .process(&r, &e)
        .expect_err("two bytes lie past the high region");
    assert_eq!(err, CommitError::OutOfRange { effect_index: 1 });
    assert_eq!(bed.memory().read(range(0, 4)).as_deref().unwrap(), &[0; 4]);
}
