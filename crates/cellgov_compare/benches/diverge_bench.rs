//! Benchmarks for the diverge scanner.
//!
//! Measures the cost of cellgov_compare::diverge over a large
//! state-trace stream so a regression in the scanner shows up
//! independently from the per-step emission cost (which has its
//! own bench in cellgov_ppu).

#![allow(missing_docs)]

use cellgov_compare::diverge;
use cellgov_trace::{StateHash, TraceRecord, TraceWriter};
use criterion::{black_box, criterion_group, criterion_main, Criterion};

/// Build a trace byte buffer of N PpuStateHash records with monotonic
/// step indices. Each record is 25 bytes, so N=100_000 produces a
/// ~2.4 MB buffer (representative of a short fault-driven boot).
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
    // Worst-case-fast: divergence is at step 0, scanner returns
    // immediately. Pairs with the identical bench to bracket the
    // scanner's per-record overhead.
    let trace = synth_trace(100_000);
    let mut other = synth_trace(100_000);
    // Mutate the first record's hash byte so step 0 diverges.
    other[1 + 8 + 8] ^= 0xFF;
    c.bench_function("diverge/diverge_at_step_0_100k_records", |b| {
        b.iter(|| diverge(black_box(&trace), black_box(&other)))
    });
}

fn bench_diverge_at_midpoint(c: &mut Criterion) {
    // Realistic: divergence at the midpoint of a 100k-record trace.
    // Forces the scanner to walk 50k records before reporting.
    let n = 100_000u64;
    let trace = synth_trace(n);
    let mut other = synth_trace(n);
    let midpoint_record_offset: usize = (n as usize / 2) * 25;
    // Flip a hash byte in the midpoint record's hash field.
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
