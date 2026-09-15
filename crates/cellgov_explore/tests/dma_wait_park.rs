//! A wake conflicts with the step that parked its unit on a transfer.
//!
//! Two of the commit pipeline's three `BlockReason` parks emit an
//! effect. The third does not: the pipeline parks a unit that yields
//! `DmaWait` from the step result alone. A footprint built from the
//! effect list alone therefore records no park. A wake that names
//! that unit then reads as independent of the step that parked it.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_exec::YieldReason;
use cellgov_explore::execution::Execution;
use cellgov_explore::observer::observe_decisions;
use cellgov_explore::util::StopReason;
use cellgov_explore::{explore_optimal, ExplorationConfig, StepFootprint};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

const PARKER: UnitId = UnitId::new(0);
const WAKER: UnitId = UnitId::new(1);

/// The transfer's two ends, clear of the word the parker stores.
const DMA_SRC: u64 = 64;
const DMA_DST: u64 = 128;
const STORED: u64 = 0;

/// One unit puts a transfer and parks on it; another wakes that unit.
///
/// The wake lands before or after the park, depending on the
/// schedule. That decides whether the parker resumes off the wake or
/// off the transfer's completion.
fn park_against_a_wake() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(1), 200);
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::DmaPut {
                    src: DMA_SRC,
                    dst: DMA_DST,
                    len: 4,
                },
                FakeOp::DmaWait,
                FakeOp::SharedStore {
                    addr: STORED,
                    len: 4,
                },
                FakeOp::End,
            ],
        )
    });
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(id, vec![FakeOp::Wake { unit: PARKER.raw() }, FakeOp::End])
    });
    rt
}

/// The exhaustive match in `parks_without_an_effect` makes a new
/// variant pick a side. A wrong side prunes the step that parked a
/// unit, and nothing else reports it.
#[test]
fn a_dma_wait_is_the_only_yield_that_parks_with_no_effect() {
    use strum::VariantArray;
    let parking: Vec<YieldReason> = YieldReason::VARIANTS
        .iter()
        .copied()
        .filter(|reason| reason.parks_without_an_effect())
        .collect();
    assert_eq!(parking, vec![YieldReason::DmaWait]);
}

#[test]
fn a_dma_wait_records_the_unit_it_parks() {
    let parked = StepFootprint::from_step(PARKER, YieldReason::DmaWait, &[]);
    assert_eq!(
        parked.wait_units,
        vec![PARKER],
        "the park reaches the footprint from the step result, with no effect to read",
    );
    assert!(
        StepFootprint::from_effects(&[]).wait_units.is_empty(),
        "the effect list alone carries no park, which is the gap from_step closes",
    );
}

#[test]
fn a_wake_conflicts_with_the_dma_wait_that_parked_the_same_unit() {
    let wake = StepFootprint::from_effects(&[Effect::WakeUnit {
        target: PARKER,
        source: WAKER,
    }]);
    let parked = StepFootprint::from_step(PARKER, YieldReason::DmaWait, &[]);
    assert!(wake.conflicts(&parked));
    assert!(
        parked.conflicts(&wake),
        "the pair conflicts from either side",
    );

    let other = StepFootprint::from_step(UnitId::new(7), YieldReason::DmaWait, &[]);
    assert!(
        !wake.conflicts(&other),
        "the wake reaches only the unit it names",
    );
}

#[test]
fn the_run_orders_the_wake_against_the_park() {
    let mut rt = park_against_a_wake();
    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::Stalled, "the workload runs itself out");
    assert!(
        log.points()
            .iter()
            .any(|point| point.footprint.wait_units.contains(&PARKER)),
        "some step of the run recorded the park of the unit the wake names",
    );
    assert!(
        !Execution::from_log(&log).units_independent(PARKER, WAKER),
        "the wake decides what the parked unit runs next, so the order matters",
    );
}

/// The optimal search is the second driver that builds a footprint;
/// `observe_decisions` above covers the first. Without the park
/// recorded, the search reads no race between the wake and the park,
/// so the two classes below collapse into one.
#[test]
fn the_optimal_search_covers_both_orders_of_the_wake_and_the_park() {
    let result = explore_optimal(
        park_against_a_wake,
        &ExplorationConfig {
            max_schedules: 1_000,
            max_steps_per_run: 10_000,
        },
    );
    assert!(
        !result.baseline_stop.is_truncated(),
        "the baseline runs the workload out: {}",
        result.baseline_stop,
    );
    assert_eq!(
        (result.schedules.len() + 1, result.classes_explored),
        (2, Some(2)),
        "one execution per class, and the wake against the park is the race \
         that makes the second one a class",
    );
}
