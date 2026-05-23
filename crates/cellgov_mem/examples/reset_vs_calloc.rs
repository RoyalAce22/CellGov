//! Microbench: `GuestMemory::reset_for_reuse` vs constructing a
//! fresh `GuestMemory` over the canonical 1.78 GiB PS3 layout
//! (1 GiB main + 1 MiB primary stack + 15 MiB child stacks + 256
//! MiB RSX + 512 MiB SPU reserved). The "after a small workload"
//! measurement writes a few KiB at typical guest VAs so reset cost
//! reflects touched-page count rather than total region size.

#![allow(clippy::unwrap_used, clippy::print_stdout, missing_docs)]

use std::time::Instant;

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region, RegionAccess};

// Constants mirror `cellgov_ps3_abi::process_address_space` so this
// example does not pull the ABI crate into `cellgov_mem`'s
// dev-deps.
//
//   STACK_BASE         = PS3_PRIMARY_STACK_BASE  = 0xD000_0000
//   STACK_SIZE         = PS3_PRIMARY_STACK_SIZE  = 0x0010_0000
//   CHILD_STACKS_BASE  = PS3_CHILD_STACKS_BASE   = 0xD010_0000
//   CHILD_STACKS_SIZE  = PS3_CHILD_STACKS_SIZE   = 0x00F0_0000
//   RSX_BASE           = PS3_RSX_BASE            = 0xC000_0000
//   RSX_SIZE           = PS3_RSX_SIZE            = 0x1000_0000
//   SPU_RESERVED_BASE  = PS3_SPU_RESERVED_BASE   = 0xE000_0000
//   SPU_RESERVED_SIZE  = PS3_SPU_RESERVED_SIZE   = 0x2000_0000
//   USER_TEXT_FLOOR    = PS3_USER_TEXT_FLOOR     = 0x0001_0000
//
// `MAIN_SIZE` matches `apps/cellgov_cli/src/game/boot.rs:117`'s
// `min_for_kernel = 0x4000_0000usize` literal; not in the ABI crate.
const MAIN_BASE: u64 = 0;
const MAIN_SIZE: usize = 0x4000_0000; // 1 GiB, mirrors boot.rs min_for_kernel
const STACK_BASE: u64 = 0xD000_0000;
const STACK_SIZE: usize = 0x0010_0000; // 1 MiB
const CHILD_STACKS_BASE: u64 = 0xD010_0000;
const CHILD_STACKS_SIZE: usize = 0x00F0_0000; // 15 MiB
const RSX_BASE: u64 = 0xC000_0000;
const RSX_SIZE: usize = 0x1000_0000; // 256 MiB
const SPU_RESERVED_BASE: u64 = 0xE000_0000;
const SPU_RESERVED_SIZE: usize = 0x2000_0000; // 512 MiB
const USER_TEXT_FLOOR: u64 = 0x0001_0000;

fn alloc_canonical() -> GuestMemory {
    GuestMemory::from_regions(vec![
        Region::new(MAIN_BASE, MAIN_SIZE, "main", PageSize::Page64K),
        Region::new(STACK_BASE, STACK_SIZE, "stack", PageSize::Page4K),
        Region::new(
            CHILD_STACKS_BASE,
            CHILD_STACKS_SIZE,
            "child_stacks",
            PageSize::Page4K,
        ),
        Region::with_access(
            RSX_BASE,
            RSX_SIZE,
            "rsx",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        ),
        Region::with_access(
            SPU_RESERVED_BASE,
            SPU_RESERVED_SIZE,
            "spu_reserved",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap()
}

/// Simulate a ps3autotests-sized workload: write a handful of KiB
/// at a handful of pages. Footprint stays small so the
/// reset-vs-calloc ratio reflects "most pages clean."
fn simulate_workload(mem: &mut GuestMemory) {
    let payload = [0xAAu8; 256];
    // Stub region near 0 (callback trampoline).
    let r = ByteRange::new(GuestAddr::new(0x100), 256).unwrap();
    mem.apply_commit(r, &payload).unwrap();
    // User-text floor.
    let r = ByteRange::new(GuestAddr::new(USER_TEXT_FLOOR), 256).unwrap();
    mem.apply_commit(r, &payload).unwrap();
    // Heap-ish.
    let r = ByteRange::new(GuestAddr::new(0x1041_0000), 256).unwrap();
    mem.apply_commit(r, &payload).unwrap();
    // Primary stack tail.
    let r = ByteRange::new(GuestAddr::new(STACK_BASE + 0x80), 256).unwrap();
    mem.apply_commit(r, &payload).unwrap();
}

fn time_ms<F: FnOnce()>(f: F) -> f64 {
    let t = Instant::now();
    f();
    t.elapsed().as_secs_f64() * 1000.0
}

fn main() {
    let warmup = alloc_canonical();
    std::hint::black_box(warmup);

    println!("=== Cold alloc (canonical 1.78 GiB layout) ===");
    let mut samples = Vec::new();
    for _ in 0..5 {
        let ms = time_ms(|| {
            let m = alloc_canonical();
            std::hint::black_box(m);
        });
        samples.push(ms);
    }
    println!("  samples (ms): {samples:?}");
    let median = {
        let mut s = samples.clone();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        s[s.len() / 2]
    };
    println!("  median: {median:.2} ms");

    println!();
    println!("=== content_hash on cold canonical layout ===");
    // content_hash skips clean pages; cost is bounded by the dirty
    // set, not the total region size.
    let mut hash_cold_samples = Vec::new();
    for _ in 0..5 {
        let mem = alloc_canonical();
        let ms = time_ms(|| {
            std::hint::black_box(mem.content_hash());
        });
        hash_cold_samples.push(ms);
    }
    println!("  samples (ms): {hash_cold_samples:?}");

    println!();
    println!("=== content_hash after small workload (a few dirty pages) ===");
    let mut hash_dirty_samples = Vec::new();
    for _ in 0..5 {
        let mut mem = alloc_canonical();
        simulate_workload(&mut mem);
        let ms = time_ms(|| {
            std::hint::black_box(mem.content_hash());
        });
        hash_dirty_samples.push(ms);
    }
    println!("  samples (ms): {hash_dirty_samples:?}");

    println!();
    println!("=== Workload + reset_for_reuse (reuse path) ===");
    let mut mem = alloc_canonical();
    simulate_workload(&mut mem);
    let mut reset_samples = Vec::new();
    for _ in 0..20 {
        simulate_workload(&mut mem);
        let ms = time_ms(|| mem.reset_for_reuse());
        reset_samples.push(ms);
    }
    println!("  samples (ms): {reset_samples:?}");
    let median_reset = {
        let mut s = reset_samples.clone();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        s[s.len() / 2]
    };
    println!("  median: {median_reset:.4} ms");

    println!();
    println!("=== Ratio ===");
    println!(
        "  reset / calloc median: {:.4} ({:.2}%)",
        median_reset / median,
        100.0 * median_reset / median
    );
}
