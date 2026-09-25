//! The four commit-checkpoint hash producers, each at several sizes, so
//! the curve of each cost against what the runtime holds is visible.

#![allow(missing_docs)]
#![allow(
    clippy::unwrap_used,
    reason = "bench scaffolding: .unwrap() panics on unexpected failure are the right behavior"
)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

use cellgov_core::Runtime;
use cellgov_exec::{FakeIsaUnit, FakeOp};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

/// Live kernel objects: a small boot to a large one.
const OBJECT_COUNTS: [usize; 5] = [0, 16, 256, 4096, 65536];

/// Registered units.
const UNIT_COUNTS: [usize; 4] = [1, 8, 64, 512];

fn runtime_with_lwmutexes(n: usize) -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(4096), Budget::new(1), 1);
    for _ in 0..n {
        rt.lv2_host_mut().lwmutexes_mut().create().unwrap();
    }
    rt
}

fn runtime_with_units(n: usize) -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(4096), Budget::new(1), 1);
    for _ in 0..n {
        rt.register_unit_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
    }
    rt
}

fn bench_lv2_host(c: &mut Criterion) {
    let mut group = c.benchmark_group("checkpoint/lv2_host_state_hash");
    for n in OBJECT_COUNTS {
        let rt = runtime_with_lwmutexes(n);
        group.bench_with_input(BenchmarkId::from_parameter(n), &rt, |b, rt| {
            b.iter(|| black_box(rt).lv2_host().state_hash())
        });
    }
    group.finish();
}

fn bench_sync_state(c: &mut Criterion) {
    let mut group = c.benchmark_group("checkpoint/sync_state_hash");
    for n in OBJECT_COUNTS {
        let rt = runtime_with_lwmutexes(n);
        group.bench_with_input(BenchmarkId::from_parameter(n), &rt, |b, rt| {
            b.iter(|| black_box(rt).sync_state_hash())
        });
    }
    group.finish();
}

fn bench_registry(c: &mut Criterion) {
    let mut status = c.benchmark_group("checkpoint/unit_status_hash");
    for n in UNIT_COUNTS {
        let rt = runtime_with_units(n);
        status.bench_with_input(BenchmarkId::from_parameter(n), &rt, |b, rt| {
            b.iter(|| black_box(rt).registry().status_hash())
        });
    }
    status.finish();
    let mut queue = c.benchmark_group("checkpoint/runnable_queue_hash");
    for n in UNIT_COUNTS {
        let rt = runtime_with_units(n);
        queue.bench_with_input(BenchmarkId::from_parameter(n), &rt, |b, rt| {
            b.iter(|| black_box(rt).registry().runnable_queue_hash())
        });
    }
    queue.finish();
}

/// One 4-byte write, then the hash: the dirty-page rehash a commit pays.
fn bench_committed_memory(c: &mut Criterion) {
    let mut group = c.benchmark_group("checkpoint/committed_memory_after_one_write");
    for size in [64 * 1024, 1024 * 1024, 16 * 1024 * 1024] {
        let mut mem = GuestMemory::new(size);
        let range = ByteRange::new(GuestAddr::new(0x100), 4).unwrap();
        let mut v = 0u32;
        group.bench_function(BenchmarkId::from_parameter(size), |b| {
            b.iter(|| {
                v = v.wrapping_add(1);
                mem.apply_commit(range, &v.to_be_bytes()).unwrap();
                black_box(mem.content_hash())
            })
        });
    }
    group.finish();
}

/// One 4-byte write, then the hash, with `n` pages already written: a
/// booted image leaves thousands of written pages, and the hash walks
/// every one of them after any write.
fn bench_committed_memory_dirty(c: &mut Criterion) {
    let mut group = c.benchmark_group("checkpoint/committed_memory_with_n_written_pages");
    for n in [1usize, 256, 2560] {
        let mut mem = GuestMemory::new(16 * 1024 * 1024);
        for page in 0..n {
            let range = ByteRange::new(GuestAddr::new((page * 4096) as u64), 4).unwrap();
            mem.apply_commit(range, &[0xa5; 4]).unwrap();
        }
        let range = ByteRange::new(GuestAddr::new(0x100), 4).unwrap();
        let mut v = 0u32;
        group.bench_function(BenchmarkId::from_parameter(n), |b| {
            b.iter(|| {
                v = v.wrapping_add(1);
                mem.apply_commit(range, &v.to_be_bytes()).unwrap();
                black_box(mem.content_hash())
            })
        });
    }
    group.finish();
}

criterion_group!(
    checkpoint_benches,
    bench_committed_memory_dirty,
    bench_lv2_host,
    bench_sync_state,
    bench_registry,
    bench_committed_memory
);
criterion_main!(checkpoint_benches);
