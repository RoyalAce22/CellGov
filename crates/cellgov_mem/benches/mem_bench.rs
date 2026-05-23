//! Memory microbenchmarks: content_hash throughput at 1 MB / 16 MB / 256 MB /
//! 1 GB, apply_commit latency, FNV-1a raw throughput, and region-lookup cost
//! on the PS3 LV2 VA layout.

#![allow(
    missing_docs,
    reason = "criterion_group! expands to pub fns that an outer doc \
              comment cannot reach"
)]
#![allow(
    clippy::unwrap_used,
    reason = "bench scaffolding: .unwrap() panics on unexpected failure are the right behavior"
)]

use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, Criterion};

use cellgov_mem::{fnv1a, ByteRange, GuestAddr, GuestMemory, PageSize, Region, RegionAccess};
use cellgov_ps3_abi::process_address_space::{
    PS3_CHILD_STACKS_BASE, PS3_CHILD_STACKS_SIZE, PS3_PRIMARY_STACK_BASE, PS3_PRIMARY_STACK_SIZE,
    PS3_RSX_BASE, PS3_RSX_SIZE, PS3_SPU_RESERVED_BASE, PS3_SPU_RESERVED_SIZE,
};

// Dirty-page tracking in `cellgov_mem::guest` is 4 KiB-granular and
// the bits are set only by `apply_commit`. A freshly-`new`'d
// `GuestMemory` has an empty bitmap, so `content_hash` over it walks
// nothing and measures only `Arc::clone` plus empty-bitmap iteration.
// The `*_dirty` benches prime 50% of pages via `apply_commit` so the
// hash walks populated state. 50% is a sensitivity-guard fraction,
// not a production-ratio figure.
const BENCH_PAGE_BYTES: usize = 4096;

// Primes 50% of `mem`'s 4 KiB pages with a non-zero pattern.
// Full-page writes (not 4-byte tags) so "50% dirty" is literal in
// bytes regardless of whether `content_hash` walks page-granular or
// byte-extent-granular regions.
fn prime_dirty_50pct(size: usize) -> GuestMemory {
    let mut mem = GuestMemory::new(size);
    let num_pages = size / BENCH_PAGE_BYTES;
    let page = [0xABu8; BENCH_PAGE_BYTES];
    for p in (0..num_pages).step_by(2) {
        let addr = (p * BENCH_PAGE_BYTES) as u64;
        let range = ByteRange::new(GuestAddr::new(addr), BENCH_PAGE_BYTES as u64).unwrap();
        mem.apply_commit(range, &page).unwrap();
    }
    mem
}

// 30 s measurement / 3 s warm-up for hash benches that take hundreds
// of ms per sample. Criterion's 5 s default cannot fit 10 samples of
// a 1 GB FNV walk.
fn configure_large_group<M: criterion::measurement::Measurement>(
    g: &mut criterion::BenchmarkGroup<'_, M>,
) {
    g.sample_size(10)
        .measurement_time(Duration::from_secs(30))
        .warm_up_time(Duration::from_secs(3));
}

// Untimed sanity check run once per dirty bench. Compute the hash,
// invalidate, recompute, and assert equality -- pins both the
// determinism invariant the cross-runner compare relies on AND the
// populated -> cleared transition `invalidate_content_hash` must
// guarantee. Leaves `mem` with cache cleared so the bench's first
// iteration walks from a known state.
fn assert_hash_deterministic_and_clear(mem: &GuestMemory) {
    let h = mem.content_hash();
    mem.invalidate_content_hash();
    assert!(!mem.is_content_hash_cached());
    assert_eq!(
        h,
        mem.content_hash(),
        "content_hash must be stable across recompute"
    );
    mem.invalidate_content_hash();
}

fn bench_content_hash_1mb_dirty(c: &mut Criterion) {
    let mem = prime_dirty_50pct(1 << 20);
    assert_hash_deterministic_and_clear(&mem);
    c.bench_function("content_hash/1mb_dirty", |b| {
        b.iter(|| {
            mem.invalidate_content_hash();
            black_box(mem.content_hash())
        })
    });
}

fn bench_content_hash_16mb_dirty(c: &mut Criterion) {
    let mem = prime_dirty_50pct(16 << 20);
    assert_hash_deterministic_and_clear(&mem);
    c.bench_function("content_hash/16mb_dirty", |b| {
        b.iter(|| {
            mem.invalidate_content_hash();
            black_box(mem.content_hash())
        })
    });
}

fn bench_content_hash_256mb_dirty(c: &mut Criterion) {
    let mem = prime_dirty_50pct(256 << 20);
    assert_hash_deterministic_and_clear(&mem);
    let mut group = c.benchmark_group("content_hash_large");
    configure_large_group(&mut group);
    group.bench_function("256mb_dirty", |b| {
        b.iter(|| {
            mem.invalidate_content_hash();
            black_box(mem.content_hash())
        })
    });
    group.finish();
}

fn bench_content_hash_1gb_dirty(c: &mut Criterion) {
    let mem = prime_dirty_50pct(1 << 30);
    assert_hash_deterministic_and_clear(&mem);
    let mut group = c.benchmark_group("content_hash_large");
    configure_large_group(&mut group);
    group.bench_function("1gb_dirty", |b| {
        b.iter(|| {
            mem.invalidate_content_hash();
            black_box(mem.content_hash())
        })
    });
    group.finish();
}

// Cache-hit cost: prime a populated memory, force the cache, then
// measure repeated `content_hash` calls. The setup `assert!`
// distinguishes "cache hit" from "empty-bitmap short-circuit".
fn bench_content_hash_cached(c: &mut Criterion) {
    let mem = prime_dirty_50pct(1 << 20);
    let _ = mem.content_hash();
    assert!(
        mem.is_content_hash_cached(),
        "priming call should populate cache"
    );
    c.bench_function("content_hash/cached", |b| {
        b.iter(|| black_box(mem.content_hash()))
    });
}

// Force the buffer's pages physically resident before timing.
// `vec![0u8; N]` is lazily backed by shared zero pages; without this
// the first hash sample pays first-touch page-fault cost that real
// dirtied guest memory never would.
fn residency_fill(size: usize) -> Vec<u8> {
    let mut data = vec![0u8; size];
    for (i, b) in data.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(31);
    }
    data
}

fn bench_fnv1a_raw_1mb(c: &mut Criterion) {
    let data = residency_fill(1 << 20);
    c.bench_function("fnv1a_raw/1mb", |b| {
        b.iter(|| black_box(fnv1a(black_box(&data))))
    });
}

fn bench_fnv1a_raw_16mb(c: &mut Criterion) {
    let data = residency_fill(16 << 20);
    c.bench_function("fnv1a_raw/16mb", |b| {
        b.iter(|| black_box(fnv1a(black_box(&data))))
    });
}

fn bench_fnv1a_raw_1gb(c: &mut Criterion) {
    let data = residency_fill(1 << 30);
    let mut group = c.benchmark_group("fnv1a_raw_large");
    configure_large_group(&mut group);
    group.bench_function("1gb", |b| b.iter(|| black_box(fnv1a(black_box(&data)))));
    group.finish();
}

// Steady-state re-dirty: writes the same range every iteration, so
// after the first iter the page bit is already set and the cache is
// already invalidated. Measures the re-write path, not first-touch.
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

// Steady-state re-dirty over a full 4 KiB page.
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

// Region-aware hot paths. Every read/write does a region lookup;
// these benches measure the PS3 LV2 layout's cost against the
// flat-buffer baseline.

// 5-region PS3 LV2 layout. Region count drives `containing_region`'s
// `partition_point` depth, so the lookup benches must run against
// the real count (5), not a subset. `rsx` and `spu_reserved` use
// `ReservedZeroReadable` to match the runtime defaults; the
// `apply_commit/*_region` benches keep their targets in the
// `ReadWrite` regions (`main`, `stack`). Listed in ascending base
// order; `from_regions` sorts internally, but the literal matches
// the sorted invariant so a reader can scan it as the layout.
//
// `main` has no ABI constant: base 0 is fixed (the PPU fetch path
// depends on `as_bytes()` starting at the region base), and size is
// per-title at runtime. 1 GiB matches the title-pressure ceiling
// that production boots build.
fn ps3_layout() -> GuestMemory {
    GuestMemory::from_regions(vec![
        Region::new(0x0000_0000, 0x4000_0000, "main", PageSize::Page64K),
        Region::with_access(
            PS3_RSX_BASE,
            PS3_RSX_SIZE,
            "rsx",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        ),
        Region::new(
            PS3_PRIMARY_STACK_BASE,
            PS3_PRIMARY_STACK_SIZE,
            "stack",
            PageSize::Page4K,
        ),
        Region::new(
            PS3_CHILD_STACKS_BASE,
            PS3_CHILD_STACKS_SIZE,
            "child_stacks",
            PageSize::Page4K,
        ),
        Region::with_access(
            PS3_SPU_RESERVED_BASE,
            PS3_SPU_RESERVED_SIZE,
            "spu_reserved",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap()
}

fn bench_containing_region_main(c: &mut Criterion) {
    let mem = ps3_layout();
    c.bench_function("containing_region/main", |b| {
        b.iter(|| black_box(mem.containing_region(black_box(0x0010_0000), 4)))
    });
}

fn bench_containing_region_stack(c: &mut Criterion) {
    let mem = ps3_layout();
    c.bench_function("containing_region/stack", |b| {
        b.iter(|| black_box(mem.containing_region(black_box(0xD000_FFF0), 8)))
    });
}

fn bench_containing_region_unmapped(c: &mut Criterion) {
    let mem = ps3_layout();
    c.bench_function("containing_region/unmapped", |b| {
        b.iter(|| black_box(mem.containing_region(black_box(0x8000_0000), 4)))
    });
}

// Targets `main` (ReadWrite); retargeting at `rsx` or `spu_reserved`
// would hit `MemError::ReservedWrite` and panic via `.unwrap()`.
fn bench_apply_commit_main_region(c: &mut Criterion) {
    let mut mem = ps3_layout();
    let range = ByteRange::new(GuestAddr::new(0x0010_0000), 4).unwrap();
    let data = [1, 2, 3, 4];
    c.bench_function("apply_commit/main_region/4b", |b| {
        b.iter(|| mem.apply_commit(range, black_box(&data)).unwrap())
    });
}

// Targets `stack` (ReadWrite); see `apply_commit_main_region` for the
// reserved-region caveat.
fn bench_apply_commit_stack_region(c: &mut Criterion) {
    let mut mem = ps3_layout();
    let range = ByteRange::new(GuestAddr::new(0xD000_FFF0), 8).unwrap();
    let data = [1, 2, 3, 4, 5, 6, 7, 8];
    c.bench_function("apply_commit/stack_region/8b", |b| {
        b.iter(|| mem.apply_commit(range, black_box(&data)).unwrap())
    });
}

fn bench_fault_context(c: &mut Criterion) {
    let mem = ps3_layout();
    c.bench_function("fault_context/in_gap", |b| {
        b.iter(|| black_box(mem.fault_context(black_box(0xB000_0000))))
    });
}

criterion_group!(
    hash_benches,
    bench_content_hash_1mb_dirty,
    bench_content_hash_16mb_dirty,
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

criterion_group!(
    hash_large_benches,
    bench_content_hash_256mb_dirty,
    bench_content_hash_1gb_dirty,
    bench_fnv1a_raw_1gb,
);

criterion_main!(hash_benches, region_benches, hash_large_benches);
