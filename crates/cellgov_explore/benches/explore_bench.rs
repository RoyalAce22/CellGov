//! Schedule-explorer microbenchmarks: the happens-before build and the
//! race scan over one execution.
//!
//! Both walk every earlier event of every other unit per event, so the
//! cost is quadratic in the window's steps. Two shapes bound it: units
//! that never conflict, where no conflict cuts a backward scan short,
//! and a window holding a step that rides a landing, which conflicts
//! with every step.

#![allow(missing_docs)]
#![allow(
    clippy::unwrap_used,
    reason = "bench scaffolding: .unwrap() panics on unexpected failure are the right behavior"
)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

use cellgov_event::UnitId;
use cellgov_explore::dependency::StepFootprint;
use cellgov_explore::execution::Execution;
use cellgov_mem::{ByteRange, GuestAddr};

const UNITS: u64 = 4;

fn word(slot: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(slot * 8), 4).unwrap()
}

/// `events` steps round-robin over [`UNITS`] units; each unit writes
/// its own word, so no pair conflicts.
fn disjoint_writers(events: usize) -> Execution {
    let mut execution = Execution::new();
    for index in 0..events {
        let unit = index as u64 % UNITS;
        let footprint = StepFootprint {
            shared_writes: vec![word(unit)],
            ..StepFootprint::default()
        };
        execution.push(UnitId::new(unit), footprint);
    }
    execution
}

/// [`disjoint_writers`] with one step in the middle that rides a
/// landing: a transfer in flight during it covers the word it writes,
/// so it conflicts with every step.
fn with_a_rider(events: usize) -> Execution {
    let mut execution = Execution::new();
    let rider_at = events / 2;
    for index in 0..events {
        let unit = index as u64 % UNITS;
        let mut footprint = StepFootprint {
            shared_writes: vec![word(unit)],
            ..StepFootprint::default()
        };
        if index == rider_at {
            footprint.inflight_dma_ranges.push(word(unit));
        }
        execution.push(UnitId::new(unit), footprint);
    }
    execution
}

fn bench_happens_before(c: &mut Criterion) {
    let mut group = c.benchmark_group("happens_before");
    for &events in &[512usize, 2048] {
        let disjoint = disjoint_writers(events);
        group.bench_with_input(
            BenchmarkId::new("disjoint_writers", events),
            &disjoint,
            |b, execution| b.iter(|| black_box(execution.happens_before())),
        );
        let rider = with_a_rider(events);
        group.bench_with_input(
            BenchmarkId::new("with_a_rider", events),
            &rider,
            |b, execution| b.iter(|| black_box(execution.happens_before())),
        );
    }
    group.finish();
}

fn bench_races(c: &mut Criterion) {
    let mut group = c.benchmark_group("races");
    for &events in &[512usize, 2048] {
        let disjoint = disjoint_writers(events);
        let disjoint_hb = disjoint.happens_before();
        group.bench_with_input(
            BenchmarkId::new("disjoint_writers", events),
            &(&disjoint, &disjoint_hb),
            |b, (execution, hb)| b.iter(|| black_box(execution.races(hb))),
        );
        let rider = with_a_rider(events);
        let rider_hb = rider.happens_before();
        group.bench_with_input(
            BenchmarkId::new("with_a_rider", events),
            &(&rider, &rider_hb),
            |b, (execution, hb)| b.iter(|| black_box(execution.races(hb))),
        );
    }
    group.finish();
}

criterion_group!(benches, bench_happens_before, bench_races);
criterion_main!(benches);
