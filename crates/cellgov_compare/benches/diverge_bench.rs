//! Criterion benchmarks for the diverge scanner.

#![allow(missing_docs)]

use cellgov_compare::diverge;
use cellgov_trace::{StateHash, TraceRecord, TraceWriter};
use criterion::{black_box, criterion_group, criterion_main, Criterion};

/// Each `PpuStateHash` record is 25 bytes, so N=100_000 yields ~2.4 MB.
fn synth_trace(n: u64) -> Vec<u8> {
    let mut w = TraceWriter::new();
    for i in 0..n {
        w.record(&TraceRecord::PpuStateHash {
            step: i,
            pc: 0x10000 + (i * 4),
            hash: StateHash::new(i.wrapping_mul(0x9E3779B97F4A7C15)),
        });
    }
    w.take_bytes()
}

fn bench_diverge_identical_100k(c: &mut Criterion) {
    let trace = synth_trace(100_000);
    c.bench_function("diverge/identical_100k_records", |b| {
        b.iter(|| diverge(black_box(&trace), black_box(&trace)))
    });
}

fn bench_diverge_first_step_differs(c: &mut Criterion) {
    let trace = synth_trace(100_000);
    let mut other = synth_trace(100_000);
    other[1 + 8 + 8] ^= 0xFF;
    c.bench_function("diverge/diverge_at_step_0_100k_records", |b| {
        b.iter(|| diverge(black_box(&trace), black_box(&other)))
    });
}

fn bench_diverge_at_midpoint(c: &mut Criterion) {
    let n = 100_000u64;
    let trace = synth_trace(n);
    let mut other = synth_trace(n);
    let midpoint_record_offset: usize = (n as usize / 2) * 25;
    other[midpoint_record_offset + 1 + 8 + 8] ^= 0x01;
    c.bench_function("diverge/diverge_at_midpoint_100k_records", |b| {
        b.iter(|| diverge(black_box(&trace), black_box(&other)))
    });
}

criterion_group!(
    diverge_benches,
    bench_diverge_identical_100k,
    bench_diverge_first_step_differs,
    bench_diverge_at_midpoint,
);
criterion_main!(diverge_benches);
