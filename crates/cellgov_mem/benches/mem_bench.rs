//! Memory microbenchmarks.
//!
//! Measures: content_hash throughput at 1 MB, 16 MB, 260 MB, and 1 GB;
//! apply_commit latency; FNV-1a raw throughput.
//!
//! The 1 GB size matches the worst case of `cellgov_cli run-game`,
//! which sizes guest memory to `min_for_kernel = 0x40000000` so the
//! 1 GB user region defined by the PS3 LV2 VA layout is fully
//! backed. The 260 MB intermediate point represents the upper end of
//! a typical retail EBOOT's PT_LOAD footprint; 1 GB covers full-boot
//! content_hash cost once the allocator has walked into the kernel
//! alloc region.

#![allow(missing_docs)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};

use cellgov_mem::{fnv1a, ByteRange, GuestAddr, GuestMemory, PageSize, Region};

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

fn bench_content_hash_1gb(c: &mut Criterion) {
    // 1 GB matches min_for_kernel = 0x40000000 in run-game, the worst-case
    // checkpoint hash cost when comparing two full-boot observations.
    let mem = GuestMemory::new(1 << 30); // 1 GB
    let mut group = c.benchmark_group("content_hash_large");
    group.sample_size(10);
    group.bench_function("1gb", |b| {
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

fn bench_fnv1a_raw_1gb(c: &mut Criterion) {
    // Worst case for the run-game kernel-region size.
    let data = vec![0u8; 1 << 30];
    let mut group = c.benchmark_group("fnv1a_raw_large");
    group.sample_size(10);
    group.bench_function("1gb", |b| b.iter(|| black_box(fnv1a(black_box(&data)))));
    group.finish();
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

// -- Region-aware hot paths --
//
// `GuestMemory` is backed by a sparse `BTreeMap<u64, Region>`. Every
// read/write does a region lookup. These benches measure the lookup
// cost for the layouts `run-game` builds (4 regions: main, stack,
// rsx, spu_reserved) and the worst-case multi-region commit so we
// can spot regressions against the flat-buffer baseline.

fn ps3_layout() -> GuestMemory {
    GuestMemory::from_regions(vec![
        Region::new(0, 0x4000_0000, "main", PageSize::Page64K),
        Region::new(0xD000_0000, 0x0001_0000, "stack", PageSize::Page4K),
        Region::new(0xC000_0000, 0x1000_0000, "rsx", PageSize::Page64K),
        Region::new(0xE000_0000, 0x2000_0000, "spu_reserved", PageSize::Page64K),
    ])
    .unwrap()
}

fn bench_containing_region_main(c: &mut Criterion) {
    // Hottest case: PPU loads at addresses inside the main region
    // (most game accesses).
    let mem = ps3_layout();
    c.bench_function("containing_region/main", |b| {
        b.iter(|| black_box(mem.containing_region(black_box(0x0010_0000), 4)))
    });
}

fn bench_containing_region_stack(c: &mut Criterion) {
    // Stack accesses: the PPU resolves these against the stack region
    // (BTreeMap walks past `main`).
    let mem = ps3_layout();
    c.bench_function("containing_region/stack", |b| {
        b.iter(|| black_box(mem.containing_region(black_box(0xD000_FFF0), 8)))
    });
}

fn bench_containing_region_unmapped(c: &mut Criterion) {
    // Unmapped address in the gap between main and rsx. This is the
    // cold path, but still needs to be cheap because every load of
    // every `Bcctr`-target taken from memory goes through it.
    let mem = ps3_layout();
    c.bench_function("containing_region/unmapped", |b| {
        b.iter(|| black_box(mem.containing_region(black_box(0x8000_0000), 4)))
    });
}

fn bench_apply_commit_main_region(c: &mut Criterion) {
    let mut mem = ps3_layout();
    let range = ByteRange::new(GuestAddr::new(0x0010_0000), 4).unwrap();
    let data = [1, 2, 3, 4];
    c.bench_function("apply_commit/main_region/4b", |b| {
        b.iter(|| mem.apply_commit(range, black_box(&data)).unwrap())
    });
}

fn bench_apply_commit_stack_region(c: &mut Criterion) {
    let mut mem = ps3_layout();
    let range = ByteRange::new(GuestAddr::new(0xD000_FFF0), 8).unwrap();
    let data = [1, 2, 3, 4, 5, 6, 7, 8];
    c.bench_function("apply_commit/stack_region/8b", |b| {
        b.iter(|| mem.apply_commit(range, black_box(&data)).unwrap())
    });
}

fn bench_fault_context(c: &mut Criterion) {
    // Constructed only on out-of-region faults but exercised by every
    // `Unmapped` diagnostic. Two BTreeMap range walks per call.
    let mem = ps3_layout();
    c.bench_function("fault_context/in_gap", |b| {
        b.iter(|| black_box(mem.fault_context(black_box(0xB000_0000))))
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

criterion_group!(
    region_benches,
    bench_containing_region_main,
    bench_containing_region_stack,
    bench_containing_region_unmapped,
    bench_apply_commit_main_region,
    bench_apply_commit_stack_region,
    bench_fault_context,
);

// Separate group for the slow large-memory benchmarks.
criterion_group!(
    hash_large_benches,
    bench_content_hash_260mb,
    bench_content_hash_1gb,
    bench_fnv1a_raw_1gb,
);

criterion_main!(hash_benches, region_benches, hash_large_benches);
