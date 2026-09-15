//! Read intent in the dependency relation: a read set adds Bernstein's
//! read-against-write clauses and leaves read-against-read and
//! read-against-reservation independent.

use crate::dependency::StepFootprint;
use crate::execution::Execution;
use crate::observer::observe_decisions;
use crate::{explore, ExplorationConfig, OutcomeClass};
use cellgov_core::{AddressSpaceId, Runtime};
use cellgov_dma::{DmaDirection, DmaRequest};
use cellgov_effects::payload::WritePayload;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_testkit::world::DmaSubmitter;
use cellgov_time::{Budget, GuestTicks};

/// Address unit 0 writes and unit 1 reads.
const RACED: u64 = 0x20;
/// Address unit 1 stores the value it read to. Disjoint from
/// [`RACED`], so the write-write clause alone proves independence.
const STEERED: u64 = 0x30;

fn range(start: u64, len: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(start), len).unwrap()
}

fn reads(start: u64, len: u64) -> StepFootprint {
    StepFootprint::from_effects(&[Effect::SharedReadIntent {
        range: range(start, len),
        source: UnitId::new(0),
    }])
}

fn writes(start: u64, len: u64) -> StepFootprint {
    StepFootprint::from_effects(&[Effect::shared_write(
        range(start, len),
        WritePayload::new(vec![0; len as usize]),
        UnitId::new(1),
        GuestTicks::ZERO,
    )])
}

/// The footprint of the one step in which `unit` touched shared state.
fn shared_footprint(execution: &Execution, unit: UnitId) -> StepFootprint {
    let shared: Vec<_> = execution
        .events_of(unit)
        .filter(|event| !event.footprint.is_local_only())
        .collect();
    assert_eq!(
        shared.len(),
        1,
        "unit {unit:?} touches shared state in exactly one step"
    );
    shared[0].footprint.clone()
}

fn dma(src: u64, dst: u64, len: u64) -> StepFootprint {
    let request = DmaRequest::new(
        DmaDirection::Put,
        range(src, len),
        range(dst, len),
        UnitId::new(1),
    )
    .unwrap();
    StepFootprint::from_effects(&[Effect::DmaEnqueue {
        request,
        payload: None,
    }])
}

#[test]
fn a_read_conflicts_with_an_overlapping_write() {
    assert!(reads(0, 8).conflicts(&writes(4, 8)));
}

#[test]
fn a_write_conflicts_with_an_overlapping_read() {
    assert!(writes(4, 8).conflicts(&reads(0, 8)));
}

#[test]
fn a_read_and_a_disjoint_write_are_independent() {
    assert!(!reads(0, 4).conflicts(&writes(8, 4)));
}

#[test]
fn two_reads_of_the_same_bytes_are_independent() {
    assert!(!reads(0, 8).conflicts(&reads(0, 8)));
}

#[test]
fn a_read_pairs_with_the_destination_a_transfer_writes() {
    // The landing writes the destination, so the order decides which
    // value a read of those bytes sees.
    assert!(reads(0, 8).conflicts(&dma(0x100, 4, 8)));
    assert!(dma(0x100, 4, 8).conflicts(&reads(0, 8)));
}

#[test]
fn a_read_prunes_against_the_source_a_transfer_reads() {
    // Both steps read the same bytes, and one of the reads is the
    // completion's. Neither changes what the other sees.
    assert!(!reads(0x100, 8).conflicts(&dma(0x100, 4, 8)));
    assert!(!dma(0x100, 4, 8).conflicts(&reads(0x100, 8)));
}

#[test]
fn a_write_pairs_with_either_end_of_a_transfer() {
    assert!(writes(0, 8).conflicts(&dma(0x100, 4, 8)));
    assert!(writes(0x100, 8).conflicts(&dma(0x100, 4, 8)));
}

/// The payloaded case: see [`StepFootprint::dma_reads`].
#[test]
fn a_payloaded_transfer_records_no_source() {
    let request = DmaRequest::new(
        DmaDirection::Put,
        range(0x100, 8),
        range(4, 8),
        UnitId::new(1),
    )
    .unwrap();
    let carried = StepFootprint::from_effects(&[Effect::DmaEnqueue {
        request,
        payload: Some(vec![0u8; 8]),
    }]);
    assert!(carried.dma_reads.is_empty());
    assert_eq!(carried.dma_writes, vec![range(4, 8)]);
    assert!(!carried.conflicts(&writes(0x100, 8)));
    assert!(!carried.conflicts(&reads(0x100, 8)));
}

/// The queue is where `note_inflight` takes the payload flag.
#[test]
fn an_unpayloaded_flight_records_both_of_its_ends() {
    let source = range(0, 4);
    let destination = range(128, 4);
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(2), 16);
    rt.register_unit_with(|id| {
        DmaSubmitter::new(id, source, destination, vec![0xde, 0xad, 0xbe, 0xef])
    });
    let step = rt.step().expect("the submitter is the only runnable unit");
    rt.commit_step(&step.result, &step.effects)
        .expect("the enqueue commits");

    let mut footprint = StepFootprint::from_effects(&step.effects);
    footprint.note_inflight(&rt);
    let starts: Vec<u64> = footprint
        .inflight_dma_ranges
        .iter()
        .map(|r| r.start().raw())
        .collect();
    assert!(
        starts.contains(&0) && starts.contains(&128),
        "in-flight set {starts:#x?} lacks one end of a payload-less transfer",
    );
}

/// Pins the order inside [`StepFootprint::note_commit`]: widened first,
/// the flight stays unwidened.
#[test]
fn an_inflight_landing_reaches_the_sibling_view() {
    let mut rt = runtime_with_two_views_of_one_mapping(0x40);
    let source = range(0, 4);
    // Into the first view, whose sibling sits at 0x5000.
    let destination = range(0x2010, 4);
    let unit = rt.register_unit_with(|id| {
        DmaSubmitter::new(id, source, destination, vec![0xde, 0xad, 0xbe, 0xef])
    });
    let step = rt.step().expect("the submitter is the only runnable unit");
    rt.commit_step(&step.result, &step.effects)
        .expect("the enqueue commits");

    let mut footprint = StepFootprint::from_effects(&step.effects);
    footprint.note_commit(&rt, unit);

    let starts: Vec<u64> = footprint
        .inflight_dma_ranges
        .iter()
        .map(|r| r.start().raw())
        .collect();
    assert!(
        starts.contains(&0x2010),
        "the landing itself is missing: {starts:#x?}",
    );
    assert!(
        starts.contains(&0x5010),
        "the sibling view of the landing is missing, so a unit reading \
         it would prune against the flight: {starts:#x?}",
    );
}

#[test]
fn a_read_does_not_conflict_with_a_reservation_on_the_bytes_it_reads() {
    let held = StepFootprint::from_effects(&[Effect::ReservationAcquire {
        line_addr: 0,
        source: UnitId::new(2),
    }]);
    assert!(!reads(0, 8).conflicts(&held));
    assert!(!held.conflicts(&reads(0, 8)));
    // A write to the same line conflicts, so only the read side
    // prunes.
    assert!(writes(0, 8).conflicts(&held));
}

#[test]
fn a_footprint_holding_only_a_read_is_not_local_only() {
    assert!(!reads(0, 4).is_local_only());
}

/// Two views of one shared mapping, `size` bytes each, at
/// `0x2000` and `0x5000` in the boot space.
fn runtime_with_two_views_of_one_mapping(size: u64) -> Runtime {
    let mem = GuestMemory::from_regions(vec![Region::new(0, 0x1000, "low", PageSize::Page64K)])
        .expect("one region at zero installs");
    let mut rt = Runtime::new(mem, Budget::new(1), 64);
    rt.register_shared_mapping(
        7,
        size,
        &[
            (AddressSpaceId::BOOT, 0x2000),
            (AddressSpaceId::BOOT, 0x5000),
        ],
    )
    .expect("two disjoint views register");
    rt
}

#[test]
fn a_read_through_one_view_of_a_shared_mapping_conflicts_with_a_write_through_its_sibling() {
    let mut rt = runtime_with_two_views_of_one_mapping(0x40);
    // Nothing pairs the two view addresses but the alias expansion.
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![FakeOp::SharedLoad {
                addr: 0x2010,
                len: 4,
            }],
        )
    });
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::LoadImm(0xAA),
                FakeOp::SharedStore {
                    addr: 0x5010,
                    len: 4,
                },
            ],
        )
    });

    let (log, _) = observe_decisions(&mut rt);
    let execution = Execution::from_log(&log);
    let reader = shared_footprint(&execution, UnitId::new(0));
    let writer = shared_footprint(&execution, UnitId::new(1));
    assert!(
        reader.conflicts(&writer),
        "reader {:?} against writer {:?}",
        reader.shared_reads,
        writer.shared_writes
    );
}

#[test]
fn the_read_set_of_a_shared_view_carries_its_sibling_view_range() {
    // The previous case also conflicts through the writer's own alias
    // expansion, so this one reads the reader's set directly.
    let mut rt = runtime_with_two_views_of_one_mapping(0x40);
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![FakeOp::SharedLoad {
                addr: 0x2010,
                len: 4,
            }],
        )
    });

    let (log, _) = observe_decisions(&mut rt);
    let reader = shared_footprint(&Execution::from_log(&log), UnitId::new(0));
    let starts: Vec<u64> = reader
        .shared_reads
        .iter()
        .map(|r| r.start().raw())
        .collect();
    assert!(
        starts.contains(&0x2010) && starts.contains(&0x5010),
        "read set {starts:#x?} lacks one of the two views"
    );
    assert!(
        reader.shared_reads.iter().all(|r| r.length() == 4),
        "an alias keeps the read's length: {:?}",
        reader.shared_reads
    );
}

#[test]
fn a_read_of_one_view_alone_does_not_conflict_with_a_write_outside_the_mapping() {
    let mut rt = runtime_with_two_views_of_one_mapping(0x40);
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![FakeOp::SharedLoad {
                addr: 0x2010,
                len: 4,
            }],
        )
    });
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::LoadImm(0xAA),
                FakeOp::SharedStore { addr: 0, len: 4 },
            ],
        )
    });

    let (log, _) = observe_decisions(&mut rt);
    let execution = Execution::from_log(&log);
    let reader = shared_footprint(&execution, UnitId::new(0));
    let writer = shared_footprint(&execution, UnitId::new(1));
    assert!(
        !reader.conflicts(&writer),
        "the alias expansion must not reach a range outside the mapping"
    );
}

/// Unit 0 writes [`RACED`]; unit 1 reads it and stores what it read
/// to [`STEERED`].
fn write_read_race_runtime() -> Runtime {
    let mem = GuestMemory::new(0x100);
    let mut rt = Runtime::new(mem, Budget::new(1), 64);
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::LoadImm(0xAA),
                FakeOp::SharedStore {
                    addr: RACED,
                    len: 1,
                },
                FakeOp::End,
            ],
        )
    });
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::SharedLoad {
                    addr: RACED,
                    len: 1,
                },
                FakeOp::SharedStore {
                    addr: STEERED,
                    len: 1,
                },
                FakeOp::End,
            ],
        )
    });
    rt
}

#[test]
fn the_write_read_race_pair_conflicts_on_the_read_set_alone() {
    let mut rt = write_read_race_runtime();
    let (log, _) = observe_decisions(&mut rt);
    let execution = Execution::from_log(&log);
    let writer = shared_footprint(&execution, UnitId::new(0));
    let reader: Vec<StepFootprint> = execution
        .events_of(UnitId::new(1))
        .filter(|event| !event.footprint.is_local_only())
        .map(|event| event.footprint.clone())
        .collect();

    assert_eq!(reader.len(), 2, "the reading unit loads, then stores");
    assert!(
        writer.conflicts(&reader[0]),
        "the read of the raced bytes is what makes the pair conflict"
    );
    assert!(
        !writer.conflicts(&reader[1]),
        "the two units' writes are disjoint, so the write set alone proves independence"
    );
}

#[test]
fn the_write_read_race_is_explored_and_reported_schedule_sensitive() {
    let config = ExplorationConfig {
        max_schedules: 32,
        max_steps_per_run: 64,
    };
    let result =
        explore(write_read_race_runtime, &config).expect("the run holds a branching point");
    assert_eq!(
        result.schedules_pruned, 0,
        "no alternate may prune: every pair conflicts through the read set"
    );
    assert_eq!(
        result.outcome,
        OutcomeClass::ScheduleSensitive,
        "the steered store lands a different byte depending on the order: {result:?}"
    );
}
