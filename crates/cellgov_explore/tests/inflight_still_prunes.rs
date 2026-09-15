//! A transfer in flight does not make every step conflict.
//!
//! A step taken while a transfer is in flight decides where that
//! transfer lands. That matters to a step which accesses the bytes the
//! transfer reads or writes, and to nothing else. So the rule names
//! the transfer's ranges against the other step's accesses.
//!
//! The workload here has a transfer in flight over the whole run and
//! no other unit that touches either end of it. Nothing extra
//! conflicts, and the search keeps its reduction. The last case holds
//! the other half of the rule: a reservation on the line the transfer
//! lands on is an access, so the rule pairs its holder.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_explore::execution::Execution;
use cellgov_explore::observer::observe_decisions;
use cellgov_explore::util::StopReason;
use cellgov_explore::{explore_optimal, ExplorationConfig, StepFootprint};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_testkit::world::DmaSubmitter;
use cellgov_time::Budget;

/// The transfer's two ends. Neither writer below reads or writes
/// either, and the submitter's own seed write is outside the window.
const DMA_SRC: u64 = 0;
const DMA_DST: u64 = 128;

/// Each writer's own word, clear of the transfer and of each other.
const FIRST_WORD: u64 = 32;
const SECOND_WORD: u64 = 64;

const SUBMITTER: UnitId = UnitId::new(0);
const FIRST: UnitId = UnitId::new(1);
const SECOND: UnitId = UnitId::new(2);

fn writer(id: UnitId, addr: u64) -> FakeIsaUnit {
    FakeIsaUnit::new(
        id,
        vec![
            FakeOp::LoadImm(0xAA),
            FakeOp::SharedStore { addr, len: 4 },
            FakeOp::End,
        ],
    )
}

/// One transfer in flight, and two writers that share nothing.
///
/// The window opens after the submitter's enqueue step, which is why
/// the builder takes that step itself. That step's own `dma_ranges`
/// are the flight's ranges, so it conflicts with every step of the
/// flight. Its position stamps the landing tick, so that dependency
/// is real. The cases below measure the other steps.
fn two_writers_beside_a_transfer() -> Runtime {
    let src = ByteRange::new(GuestAddr::new(DMA_SRC), 4).unwrap();
    let dst = ByteRange::new(GuestAddr::new(DMA_DST), 4).unwrap();
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(2), 400);
    rt.register_unit_with(|id| DmaSubmitter::new(id, src, dst, vec![0xde, 0xad, 0xbe, 0xef]));
    let first = rt.register_unit_with(|id| writer(id, FIRST_WORD));
    let second = rt.register_unit_with(|id| writer(id, SECOND_WORD));
    // The independence check names units by id, so a registration
    // inserted above would retarget it.
    assert_eq!(
        (first, second),
        (FIRST, SECOND),
        "registration order moved the writers",
    );
    let step = rt.step().expect("the submitter runs first");
    assert_eq!(
        step.unit, SUBMITTER,
        "registration order moved the submitter"
    );
    rt.commit_step(&step.result, &step.effects)
        .expect("the enqueue commits");
    rt
}

#[test]
fn a_transfer_in_flight_does_not_make_disjoint_writers_conflict() {
    let mut rt = two_writers_beside_a_transfer();
    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::Stalled);

    // The premise: the steps really did run with a transfer in flight.
    // Without this the case would pass on a workload where the rule
    // was never asked.
    assert!(
        log.points()
            .iter()
            .any(|point| !point.footprint.inflight_dma_ranges.is_empty()),
        "some step ran while the transfer was in flight",
    );

    let execution = Execution::from_log(&log);
    assert!(
        execution.units_independent(FIRST, SECOND),
        "the two writers touch disjoint words and neither touches the transfer",
    );
}

/// Two independent writers offer one class, and a rule that made
/// every step of a flight conflict would offer two.
#[test]
fn the_search_runs_one_class_over_the_disjoint_writers() {
    let result = explore_optimal(two_writers_beside_a_transfer, &ExplorationConfig::default());
    assert_eq!(result.baseline_stop, StopReason::Stalled);
    assert_eq!(
        (result.schedules.len(), result.classes_explored),
        (0, Some(1)),
        "nothing conflicts, so the search owes no reversal and covers the one class",
    );
}

/// The landing sweeps every other unit's entry on that line, so where
/// it lands decides the next conditional store's verdict.
#[test]
fn an_inflight_landing_pairs_with_a_reservation_on_the_line_it_covers() {
    let dst = ByteRange::new(GuestAddr::new(DMA_DST), 4).unwrap();
    let during = StepFootprint {
        inflight_dma_ranges: vec![dst],
        ..StepFootprint::default()
    };
    let holder = StepFootprint::from_effects(&[Effect::ReservationAcquire {
        line_addr: DMA_DST,
        source: FIRST,
    }]);
    assert!(during.conflicts(&holder));
    assert!(holder.conflicts(&during), "the rule reads both directions");

    let elsewhere = StepFootprint::from_effects(&[Effect::ReservationAcquire {
        line_addr: DMA_DST + 128,
        source: FIRST,
    }]);
    assert!(
        !during.conflicts(&elsewhere),
        "a reservation on another line keeps its entry whatever the landing",
    );
}
