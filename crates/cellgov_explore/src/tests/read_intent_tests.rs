//! Read intent in the dependency relation.
//!
//! A read set adds two of Bernstein's clauses: read against write,
//! and write against read. It leaves two pairs independent: read
//! against read, and read against a reservation line. The cases below
//! cover those four pairs, the alias expansion over a shared mapping,
//! and a write-read race whose two orders leave different bytes.

use crate::dependency::StepFootprint;
use crate::observer::observe_decisions;
use crate::{explore, ExplorationConfig, OutcomeClass};
use cellgov_core::{AddressSpaceId, Runtime};
use cellgov_dma::{DmaDirection, DmaRequest};
use cellgov_effects::payload::WritePayload;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
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
fn a_read_conflicts_with_an_overlapping_dma_range() {
    assert!(reads(0, 8).conflicts(&dma(0x100, 4, 8)));
    assert!(reads(0x100, 8).conflicts(&dma(0x100, 4, 8)));
}

#[test]
fn a_dma_range_conflicts_with_an_overlapping_read() {
    assert!(dma(0x100, 4, 8).conflicts(&reads(0, 8)));
    assert!(dma(0x100, 4, 8).conflicts(&reads(0x100, 8)));
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

#[test]
fn merge_carries_the_read_set() {
    let mut a = reads(0, 4);
    a.merge(&reads(0x80, 4));
    assert_eq!(a.shared_reads.len(), 2);
    assert!(a.conflicts(&writes(0x80, 4)));
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
    let reader = log
        .aggregate_footprint(UnitId::new(0))
        .expect("the reading unit ran");
    let writer = log
        .aggregate_footprint(UnitId::new(1))
        .expect("the writing unit ran");
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
    let reader = log
        .aggregate_footprint(UnitId::new(0))
        .expect("the reading unit ran");
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
    let reader = log.aggregate_footprint(UnitId::new(0)).unwrap();
    let writer = log.aggregate_footprint(UnitId::new(1)).unwrap();
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
    let writer = log.aggregate_footprint(UnitId::new(0)).unwrap();
    let reader = log.aggregate_footprint(UnitId::new(1)).unwrap();

    let mut writes_only = reader.clone();
    writes_only.shared_reads.clear();
    assert!(
        !writer.conflicts(&writes_only),
        "the two units' writes are disjoint, so the write set alone proves independence"
    );
    assert!(
        writer.conflicts(&reader),
        "the read of the raced bytes is what makes the pair conflict"
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
