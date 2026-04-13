//! Memory microbenchmarks.
//!
//! Measures: content_hash throughput at 1 MB, 16 MB, and 260 MB;
//! apply_commit latency; FNV-1a raw throughput.

use criterion::{black_box, criterion_group, criterion_main, Criterion};

use cellgov_mem::{fnv1a, ByteRange, GuestAddr, GuestMemory};

fn bench_content_hash_1mb(c: &mut Criterion) {
    let mem = GuestMemory::new(1 << 20); // 1 MB
    c.bench_function("content_hash/1mb", |b| {
        b.iter(|| {
            // Invalidate cache by cloning (the clone has a fresh cache).
            let m = mem.clone();
            black_box(m.content_hash())
        })
    });
}

fn bench_content_hash_16mb(c: &mut Criterion) {
    let mem = GuestMemory::new(16 << 20); // 16 MB
    c.bench_function("content_hash/16mb", |b| {
        b.iter(|| {
            let m = mem.clone();
            black_box(m.content_hash())
        })
    });
}

fn bench_content_hash_260mb(c: &mut Criterion) {
    let mem = GuestMemory::new(260 << 20); // 260 MB
    let mut group = c.benchmark_group("content_hash_large");
    group.sample_size(10); // fewer samples for the 260 MB case
    group.bench_function("260mb", |b| {
        b.iter(|| {
            let m = mem.clone();
            black_box(m.content_hash())
        })
    });
    group.finish();
}

fn bench_content_hash_cached(c: &mut Criterion) {
    let mem = GuestMemory::new(1 << 20); // 1 MB
                                         // Warm the cache
    let _ = mem.content_hash();
    c.bench_function("content_hash/1mb_cached", |b| {
        b.iter(|| black_box(mem.content_hash()))
    });
}

fn bench_fnv1a_raw_1mb(c: &mut Criterion) {
    let data = vec![0u8; 1 << 20];
    c.bench_function("fnv1a_raw/1mb", |b| {
        b.iter(|| black_box(fnv1a(black_box(&data))))
    });
}

fn bench_fnv1a_raw_16mb(c: &mut Criterion) {
    let data = vec![0u8; 16 << 20];
    c.bench_function("fnv1a_raw/16mb", |b| {
        b.iter(|| black_box(fnv1a(black_box(&data))))
    });
}

fn bench_apply_commit_4b(c: &mut Criterion) {
    let mut mem = GuestMemory::new(4096);
    let range = ByteRange::new(GuestAddr::new(0x100), 4).unwrap();
    let data = [0xDE, 0xAD, 0xBE, 0xEF];
    c.bench_function("apply_commit/4b", |b| {
        b.iter(|| {
            mem.apply_commit(range, black_box(&data)).unwrap();
        })
    });
}

fn bench_apply_commit_4kb(c: &mut Criterion) {
    let mut mem = GuestMemory::new(1 << 20);
    let range = ByteRange::new(GuestAddr::new(0), 4096).unwrap();
    let data = vec![0xABu8; 4096];
    c.bench_function("apply_commit/4kb", |b| {
        b.iter(|| {
            mem.apply_commit(range, black_box(&data)).unwrap();
        })
    });
}

criterion_group!(
    hash_benches,
    bench_content_hash_1mb,
    bench_content_hash_16mb,
    bench_content_hash_cached,
    bench_fnv1a_raw_1mb,
    bench_fnv1a_raw_16mb,
    bench_apply_commit_4b,
    bench_apply_commit_4kb,
);

// Separate group for the slow 260 MB benchmark
criterion_group!(hash_large_benches, bench_content_hash_260mb);

criterion_main!(hash_benches, hash_large_benches);
