//! The PPU load path, split into its parts and run in three loop shapes.
//!
//! A load runs these parts in order:
//!
//! 1. It asks the store buffer to forward it.
//! 2. It scans the region views for the one that holds it, and logs a
//!    read of a provisional view.
//! 3. It overlays the buffered stores on the bytes it read.
//!
//! The `load_parts` group times each part alone. The `load_loops` group
//! runs the loop shapes that a per-site region cache would see.
#![allow(
    missing_docs,
    reason = "criterion_group! expands to pub fns that an outer doc \
              comment cannot reach"
)]
#![allow(
    clippy::unwrap_used,
    reason = "bench scaffolding: .unwrap() panics on unexpected failure are the right behavior"
)]

use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};

use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region, RegionAccess};
use cellgov_ppu::exec::execute;
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::shadow::PredecodedShadow;
use cellgov_ppu::state::PpuState;
use cellgov_ppu::store_buffer::StoreBuffer;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::Budget;

/// A store buffer with `n` word stores, none of them at 0x1000.
fn buffer_of(n: usize) -> StoreBuffer {
    let mut buf = StoreBuffer::new();
    for i in 0..n {
        buf.insert(0x4000 + 8 * i as u64, 4, i as u128).unwrap();
    }
    buf
}

/// `StoreBuffer::forward` that finds nothing, over an empty and a full
/// buffer.
fn bench_forward_miss(c: &mut Criterion) {
    for n in [0, 63] {
        let buf = buffer_of(n);
        c.bench_function(&format!("load_parts/forward_miss_{n}"), |b| {
            b.iter(|| buf.forward(black_box(0x1000), 4))
        });
    }
}

/// `StoreBuffer::overlay_range` over four bytes no store covers.
fn bench_overlay(c: &mut Criterion) {
    for n in [0, 63] {
        let buf = buffer_of(n);
        c.bench_function(&format!("load_parts/overlay_{n}"), |b| {
            let mut out = [0u8; 4];
            b.iter(|| buf.overlay_range(black_box(0x1000), &mut out))
        });
    }
}

/// One `lwz` at 0x1000 over `views`, with an empty store buffer.
fn bench_lwz_over(c: &mut Criterion, name: &str, views: &[cellgov_mem::RegionView<'_>]) {
    let insn = PpuInstruction::Lwz {
        rt: 3,
        ra: 1,
        imm: 0,
    };
    let uid = UnitId::new(0);
    c.bench_function(name, |b| {
        let mut state = PpuState::new();
        state.set_gpr(1, 0x1000);
        let mut effects = Vec::new();
        let mut store_buf = StoreBuffer::new();
        b.iter(|| {
            effects.clear();
            execute(
                black_box(&insn),
                &mut state,
                uid,
                views,
                &mut effects,
                &mut store_buf,
            );
        })
    });
}

/// The region scan: one view, and the last of eight. The difference is
/// what seven more views cost the scan.
fn bench_region_scan(c: &mut Criterion) {
    let mem = vec![0u8; 0x2000];
    bench_lwz_over(
        c,
        "load_parts/lwz_first_of_1_views",
        &[cellgov_mem::RegionView::plain(0, &mem)],
    );
    // The scan walks the views in order, so the seven that miss come first.
    let blocks = vec![[0u8; 0x100]; 7];
    let mut views: Vec<_> = blocks
        .iter()
        .enumerate()
        .map(|(i, block)| cellgov_mem::RegionView::plain(0x8000 + 0x100 * i as u64, block))
        .collect();
    views.push(cellgov_mem::RegionView::plain(0, &mem));
    bench_lwz_over(c, "load_parts/lwz_last_of_8_views", &views);
}

/// A load from a provisional view, which logs the read with the memory.
fn bench_note_read(c: &mut Criterion) {
    let mem = GuestMemory::from_regions(vec![Region::with_access(
        0,
        0x2000,
        "reserved",
        PageSize::Page4K,
        RegionAccess::ReservedZeroReadable,
    )])
    .unwrap();
    let views: Vec<_> = mem.region_views().collect();
    assert!(views[0].provisional.is_some(), "the view logs its reads");
    bench_lwz_over(c, "load_parts/lwz_provisional_view", &views);
}

/// A load the store buffer forwards whole, so no view is read.
fn bench_forwarded(c: &mut Criterion) {
    let mem = vec![0u8; 0x2000];
    let views = [cellgov_mem::RegionView::plain(0, &mem)];
    let insn = PpuInstruction::Lwz {
        rt: 3,
        ra: 1,
        imm: 0,
    };
    let uid = UnitId::new(0);
    c.bench_function("load_parts/lwz_forwarded", |b| {
        let mut state = PpuState::new();
        state.set_gpr(1, 0x1000);
        let mut effects = Vec::new();
        let mut store_buf = StoreBuffer::new();
        store_buf.insert(0x1000, 4, 7).unwrap();
        b.iter(|| {
            effects.clear();
            execute(
                black_box(&insn),
                &mut state,
                uid,
                &views,
                &mut effects,
                &mut store_buf,
            );
        })
    });
}

const TEXT_SIZE: usize = 0x1000;
/// Where the TOC-style base points, in the text region.
const TOC: u64 = 0x800;
/// The stack region, the fourth of six views as in a boot's layout.
const STACK: u64 = 0x3_0000;
/// The boundary between two adjacent regions, which the buffer walk crosses.
const BOUNDARY: u64 = 0x5_0000;

/// Six regions in a boot's order, with `code` repeated over the text region.
///
/// - text, from address 0
/// - two data regions
/// - the stack
/// - two adjacent regions that meet at `BOUNDARY`
fn loop_memory(code: &[u32]) -> GuestMemory {
    let region = |base, label| Region::new(base, 0x1_0000, label, PageSize::Page4K);
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, TEXT_SIZE, "text", PageSize::Page4K),
        region(0x1_0000, "data_a"),
        region(0x2_0000, "data_b"),
        region(STACK, "stack"),
        region(BOUNDARY - 0x1_0000, "below"),
        region(BOUNDARY, "above"),
    ])
    .unwrap();
    for i in 0..TEXT_SIZE / 4 {
        let range = ByteRange::new(GuestAddr::new(4 * i as u64), 4).unwrap();
        mem.apply_commit(range, &code[i % code.len()].to_be_bytes())
            .unwrap();
    }
    mem
}

fn d_form(op: u32, rt: u32, ra: u32, d: i16) -> u32 {
    (op << 26) | (rt << 21) | (ra << 16) | (d as u16 as u32)
}

/// 100 instructions of `code` from a fresh unit with `regs` set.
fn bench_loop(c: &mut Criterion, name: &str, code: &[u32], regs: &[(usize, u64)]) {
    let mem = loop_memory(code);
    let bytes: Vec<u8> = (0..TEXT_SIZE / 4)
        .flat_map(|i| code[i % code.len()].to_be_bytes())
        .collect();
    let shadow = PredecodedShadow::build(0, &bytes);
    c.bench_function(name, |b| {
        b.iter_batched(
            || {
                let mut ppu = PpuExecutionUnit::new(UnitId::new(0));
                ppu.set_instruction_shadow(shadow.clone());
                for &(r, v) in regs {
                    ppu.state_mut().set_gpr(r, v);
                }
                ppu
            },
            |mut ppu| {
                let ctx = ExecutionContext::new(&mem);
                let r = ppu.run_until_yield(black_box(Budget::new(100)), &ctx, &mut Vec::new());
                (ppu, r)
            },
            BatchSize::SmallInput,
        );
    });
}

/// `ld` at four offsets from r1, inside the stack region.
fn bench_stack_walk(c: &mut Criterion) {
    let code = [
        d_form(58, 3, 1, 0x10),
        d_form(58, 4, 1, 0x18),
        d_form(58, 5, 1, 0x20),
        d_form(58, 6, 1, 0x28),
    ];
    bench_loop(
        c,
        "load_loops/stack_walk_ld_r1",
        &code,
        &[(1, STACK + 0x8000)],
    );
}

/// `lwz` at four offsets from one base, as a TOC load does.
fn bench_toc_loads(c: &mut Criterion) {
    let code = [
        d_form(32, 3, 2, 0),
        d_form(32, 4, 2, 4),
        d_form(32, 5, 2, 8),
        d_form(32, 6, 2, 12),
    ];
    bench_loop(c, "load_loops/toc_lwz_r2", &code, &[(2, TOC)]);
}

/// `lwzu` steps one word at a time across the boundary of two regions.
fn bench_buffer_walk(c: &mut Criterion) {
    let code = [d_form(33, 3, 4, 4)];
    bench_loop(
        c,
        "load_loops/buffer_walk_lwzu_across_regions",
        &code,
        &[(4, BOUNDARY - 0xC8)],
    );
}

criterion_group!(
    load_parts,
    bench_forward_miss,
    bench_overlay,
    bench_region_scan,
    bench_note_read,
    bench_forwarded,
);

criterion_group!(
    load_loops,
    bench_stack_walk,
    bench_toc_loads,
    bench_buffer_walk
);

criterion_main!(load_parts, load_loops);
