//! A unit run ahead of the lowest clock runs before the events between.
//!
//! An event fires once no runnable unit's clock is below its tick. A
//! schedule that forces a unit past an event's tick while another unit
//! lags therefore runs that unit's steps before the event: the transfer
//! stays in flight until the lagging unit's clock reaches the completion
//! tick. No step sees an event early, and the step run ahead sees it
//! late.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_explore::prescribed::PrescribedScheduler;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_testkit::world::{CountingUnit, DmaSubmitter, WritingUnit};
use cellgov_time::{Budget, GuestTicks};

const SUBMITTER: UnitId = UnitId::new(0);
const WRITER: UnitId = UnitId::new(1);
const COUNTER: UnitId = UnitId::new(2);
const STEP_CAP: usize = 400;
const BUDGET: u64 = 7;
/// Enough stores to take the writer's clock past the completion tick.
const WRITER_STEPS: u64 = 7;
const COUNTER_STEPS: u64 = 8;
const TRANSFERRED: [u8; 4] = [0xde, 0xad, 0xbe, 0xef];

fn destination() -> ByteRange {
    ByteRange::new(GuestAddr::new(128), 4).expect("destination range")
}

fn workload() -> Runtime {
    let src = ByteRange::new(GuestAddr::new(0), 4).expect("source range");
    let dst = destination();
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(BUDGET), STEP_CAP);
    let submitter =
        rt.register_unit_with(|id| DmaSubmitter::new(id, src, dst, TRANSFERRED.to_vec()));
    let writer = rt.register_unit_with(|id| WritingUnit::new(id, WRITER_STEPS, dst));
    let counter = rt.register_unit_with(|id| CountingUnit::new(id, COUNTER_STEPS));
    assert_eq!(
        submitter, SUBMITTER,
        "registration order moved the submitter"
    );
    assert_eq!(writer, WRITER, "registration order moved the writer");
    assert_eq!(counter, COUNTER, "registration order moved the counter");
    rt
}

/// Run one step under the installed scheduler and commit it.
fn step(rt: &mut Runtime) {
    let step = rt.step().expect("a unit of this workload is runnable");
    rt.commit_unchecked(&step.result, &step.effects)
        .expect("no step of this workload refuses its commit");
}

fn destination_bytes(rt: &Runtime) -> Vec<u8> {
    rt.memory()
        .read(destination())
        .expect("destination mapped")
        .to_vec()
}

/// The submitter, then every store of the writer, before the counter
/// takes a step.
fn writer_ahead() -> Runtime {
    let mut prefix = vec![Some(SUBMITTER)];
    prefix.extend(std::iter::repeat_n(Some(WRITER), WRITER_STEPS as usize));
    let len = prefix.len();
    let mut rt = workload();
    rt.set_scheduler(PrescribedScheduler::new(prefix));
    for _ in 0..len {
        step(&mut rt);
    }
    rt
}

#[test]
fn a_transfer_stays_in_flight_while_a_unit_runs_past_its_completion_tick() {
    let rt = writer_ahead();
    let completion = rt
        .dma_queue()
        .next_event_time()
        .expect("the transfer is still queued");
    assert!(
        rt.registry().clock(WRITER) > completion,
        "the writer's clock passed the completion tick",
    );
    assert_eq!(
        rt.registry().clock(COUNTER),
        GuestTicks::ZERO,
        "the counter has taken no step",
    );
    assert_eq!(rt.dma_queue().len(), 1, "the transfer has not landed");
    assert_eq!(
        destination_bytes(&rt),
        [WRITER_STEPS as u8; 4],
        "the writer's last store holds the destination",
    );
}

#[test]
fn the_transfer_lands_once_the_lagging_clock_reaches_its_tick() {
    let mut rt = writer_ahead();
    let completion = rt
        .dma_queue()
        .next_event_time()
        .expect("the transfer is still queued");
    // Past the prescription the lowest clock runs, which is the counter.
    for _ in 0..COUNTER_STEPS {
        if rt.dma_queue().is_empty() {
            break;
        }
        assert!(
            rt.registry().clock(COUNTER) < completion,
            "the transfer is still in flight, so the counter is behind its tick",
        );
        step(&mut rt);
    }
    assert!(rt.dma_queue().is_empty(), "the transfer landed");
    assert!(
        rt.registry().clock(COUNTER) >= completion,
        "it landed once the counter's clock reached the completion tick",
    );
    assert_eq!(
        destination_bytes(&rt),
        TRANSFERRED,
        "the landing comes after the store run ahead of it",
    );
}
