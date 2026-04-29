//! PPU microbenchmarks: decode, execute per-variant, and `run_until_yield`.

#![allow(missing_docs)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};

use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::GuestMemory;
use cellgov_ppu::decode::decode;
use cellgov_ppu::exec::execute;
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::state::PpuState;
use cellgov_ppu::store_buffer::StoreBuffer;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::Budget;

fn bench_decode_addi(c: &mut Criterion) {
    let raw: u32 = (14 << 26) | (3 << 21) | 1;
    c.bench_function("decode/addi", |b| b.iter(|| decode(black_box(raw))));
}

fn bench_decode_lwz(c: &mut Criterion) {
    let raw: u32 = (32 << 26) | (3 << 21) | (1 << 16);
    c.bench_function("decode/lwz", |b| b.iter(|| decode(black_box(raw))));
}

fn bench_decode_stw(c: &mut Criterion) {
    let raw: u32 = (36 << 26) | (3 << 21) | (1 << 16);
    c.bench_function("decode/stw", |b| b.iter(|| decode(black_box(raw))));
}

fn bench_decode_bc(c: &mut Criterion) {
    let raw: u32 = (16 << 26) | (12 << 21) | (2 << 16) | 8;
    c.bench_function("decode/bc", |b| b.iter(|| decode(black_box(raw))));
}

fn bench_decode_xo_add(c: &mut Criterion) {
    let raw: u32 = (31 << 26) | (3 << 21) | (4 << 16) | (5 << 11) | (266 << 1);
    c.bench_function("decode/add_xo", |b| b.iter(|| decode(black_box(raw))));
}

fn bench_decode_mixed_batch(c: &mut Criterion) {
    let words: [u32; 8] = [
        (14 << 26) | (3 << 21) | 1,                                  // addi
        (32 << 26) | (3 << 21) | (1 << 16),                          // lwz
        (36 << 26) | (3 << 21) | (1 << 16),                          // stw
        (31 << 26) | (3 << 21) | (4 << 16) | (5 << 11) | (266 << 1), // add
        (15 << 26) | (3 << 21) | 1,                                  // addis
        (16 << 26) | (12 << 21) | (2 << 16) | 8,                     // bc
        (11 << 26) | (3 << 16) | 10,                                 // cmpi (cmpwi)
        (28 << 26) | (3 << 21) | (4 << 16) | 0xFF,                   // andi.
    ];
    c.bench_function("decode/mixed_batch_8", |b| {
        b.iter(|| {
            for &w in &words {
                let _ = decode(black_box(w));
            }
        })
    });
}

fn bench_execute_addi(c: &mut Criterion) {
    let insn = PpuInstruction::Addi {
        rt: 3,
        ra: 0,
        imm: 42,
    };
    let uid = UnitId::new(0);
    c.bench_function("execute/addi", |b| {
        let mut state = PpuState::new();
        let mut effects = Vec::new();
        let mut store_buf = StoreBuffer::new();
        b.iter(|| {
            effects.clear();
            store_buf.clear();
            execute(
                black_box(&insn),
                &mut state,
                uid,
                &[],
                &mut effects,
                &mut store_buf,
            );
        })
    });
}

fn bench_execute_add(c: &mut Criterion) {
    let insn = PpuInstruction::Add {
        rt: 3,
        ra: 4,
        rb: 5,
        oe: false,
        rc: false,
    };
    let uid = UnitId::new(0);
    c.bench_function("execute/add", |b| {
        let mut state = PpuState::new();
        state.gpr[4] = 100;
        state.gpr[5] = 200;
        let mut effects = Vec::new();
        let mut store_buf = StoreBuffer::new();
        b.iter(|| {
            effects.clear();
            store_buf.clear();
            execute(
                black_box(&insn),
                &mut state,
                uid,
                &[],
                &mut effects,
                &mut store_buf,
            );
        })
    });
}

fn bench_execute_lwz(c: &mut Criterion) {
    let insn = PpuInstruction::Lwz {
        rt: 3,
        ra: 1,
        imm: 0,
    };
    let uid = UnitId::new(0);
    let mem = vec![0u8; 0x2000];
    c.bench_function("execute/lwz", |b| {
        let mut state = PpuState::new();
        state.gpr[1] = 0x1000;
        let views: [(u64, &[u8]); 1] = [(0, &mem)];
        let mut effects = Vec::new();
        let mut store_buf = StoreBuffer::new();
        b.iter(|| {
            effects.clear();
            store_buf.clear();
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

fn bench_execute_stw(c: &mut Criterion) {
    let insn = PpuInstruction::Stw {
        rs: 3,
        ra: 1,
        imm: 0,
    };
    let uid = UnitId::new(0);
    c.bench_function("execute/stw", |b| {
        let mut state = PpuState::new();
        state.gpr[1] = 0x1000;
        state.gpr[3] = 0xDEAD;
        let mut effects = Vec::new();
        let mut store_buf = StoreBuffer::new();
        b.iter(|| {
            effects.clear();
            store_buf.clear();
            execute(
                black_box(&insn),
                &mut state,
                uid,
                &[],
                &mut effects,
                &mut store_buf,
            );
        })
    });
}

fn bench_execute_cmpwi(c: &mut Criterion) {
    let insn = PpuInstruction::Cmpwi {
        bf: 0,
        ra: 3,
        imm: 0,
    };
    let uid = UnitId::new(0);
    c.bench_function("execute/cmpwi", |b| {
        let mut state = PpuState::new();
        state.gpr[3] = 42;
        let mut effects = Vec::new();
        let mut store_buf = StoreBuffer::new();
        b.iter(|| {
            effects.clear();
            store_buf.clear();
            execute(
                black_box(&insn),
                &mut state,
                uid,
                &[],
                &mut effects,
                &mut store_buf,
            );
        })
    });
}

fn bench_execute_b(c: &mut Criterion) {
    let insn = PpuInstruction::B {
        offset: 0x800,
        aa: false,
        link: false,
    };
    let uid = UnitId::new(0);
    c.bench_function("execute/b", |b| {
        let mut state = PpuState::new();
        state.pc = 0x800;
        let mut effects = Vec::new();
        let mut store_buf = StoreBuffer::new();
        b.iter(|| {
            state.pc = 0x800;
            effects.clear();
            store_buf.clear();
            execute(
                black_box(&insn),
                &mut state,
                uid,
                &[],
                &mut effects,
                &mut store_buf,
            );
        })
    });
}

fn bench_execute_rlwinm(c: &mut Criterion) {
    let insn = PpuInstruction::Rlwinm {
        ra: 3,
        rs: 4,
        sh: 8,
        mb: 0,
        me: 23,
        rc: false,
    };
    let uid = UnitId::new(0);
    c.bench_function("execute/rlwinm", |b| {
        let mut state = PpuState::new();
        state.gpr[4] = 0x12345678;
        let mut effects = Vec::new();
        let mut store_buf = StoreBuffer::new();
        b.iter(|| {
            effects.clear();
            store_buf.clear();
            execute(
                black_box(&insn),
                &mut state,
                uid,
                &[],
                &mut effects,
                &mut store_buf,
            );
        })
    });
}

fn bench_run_until_yield_100(c: &mut Criterion) {
    let addi_word: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;
    let addi_bytes = addi_word.to_be_bytes();

    let mem_size = 4096;
    let mut mem = GuestMemory::new(mem_size);
    for i in 0..1000 {
        let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(i * 4), 4).unwrap();
        mem.apply_commit(range, &addi_bytes).unwrap();
    }

    c.bench_function("run_until_yield/budget_100_addi", |b| {
        b.iter(|| {
            let mut ppu = PpuExecutionUnit::new(UnitId::new(0));
            let ctx = ExecutionContext::new(&mem);
            ppu.run_until_yield(black_box(Budget::new(100)), &ctx, &mut Vec::new())
        })
    });
}

fn bench_run_until_yield_budget_1(c: &mut Criterion) {
    // Budget=1: single-step mode, matching run-game usage.
    let addi_word: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;
    let addi_bytes = addi_word.to_be_bytes();

    let mem_size = 4096;
    let mut mem = GuestMemory::new(mem_size);
    for i in 0..1000 {
        let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(i * 4), 4).unwrap();
        mem.apply_commit(range, &addi_bytes).unwrap();
    }

    c.bench_function("run_until_yield/budget_1_addi", |b| {
        b.iter(|| {
            let mut ppu = PpuExecutionUnit::new(UnitId::new(0));
            let ctx = ExecutionContext::new(&mem);
            ppu.run_until_yield(black_box(Budget::new(1)), &ctx, &mut Vec::new())
        })
    });
}

fn bench_run_until_yield_mixed(c: &mut Criterion) {
    let mut mem = GuestMemory::new(4096);
    // Straight-line sequence (no back-branch) tiled 250x; runs to budget exhaustion.
    let instructions: &[(u64, u32)] = &[
        (0x00, (14 << 26) | (3 << 21) | (3 << 16) | 1),
        (
            0x04,
            (31 << 26) | (4 << 21) | (3 << 16) | (5 << 11) | (266 << 1),
        ),
        (0x08, (11 << 26) | (3 << 16) | 100),
        (0x0C, (14 << 26) | (5 << 21) | (5 << 16) | 1),
    ];
    for rep in 0..250 {
        for &(off, word) in instructions {
            let addr = rep * 16 + off;
            let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), 4).unwrap();
            mem.apply_commit(range, &word.to_be_bytes()).unwrap();
        }
    }

    c.bench_function("run_until_yield/budget_100_mixed", |b| {
        b.iter(|| {
            let mut ppu = PpuExecutionUnit::new(UnitId::new(0));
            let ctx = ExecutionContext::new(&mem);
            ppu.run_until_yield(black_box(Budget::new(100)), &ctx, &mut Vec::new())
        })
    });
}

criterion_group!(
    decode_benches,
    bench_decode_addi,
    bench_decode_lwz,
    bench_decode_stw,
    bench_decode_bc,
    bench_decode_xo_add,
    bench_decode_mixed_batch,
);

criterion_group!(
    execute_benches,
    bench_execute_addi,
    bench_execute_add,
    bench_execute_lwz,
    bench_execute_stw,
    bench_execute_cmpwi,
    bench_execute_b,
    bench_execute_rlwinm,
);

/// Zero-cost default path: per-step state-hash trace OFF. Pairs with
/// [`bench_run_until_yield_per_step_on`] to quantify the overhead.
fn bench_run_until_yield_per_step_off(c: &mut Criterion) {
    let addi_word: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;
    let addi_bytes = addi_word.to_be_bytes();
    let mut mem = GuestMemory::new(4096);
    for i in 0..1000 {
        let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(i * 4), 4).unwrap();
        mem.apply_commit(range, &addi_bytes).unwrap();
    }

    c.bench_function("run_until_yield/per_step_off_addi100", |b| {
        b.iter(|| {
            let mut ppu = PpuExecutionUnit::new(UnitId::new(0));
            let ctx = ExecutionContext::new(&mem);
            ppu.run_until_yield(black_box(Budget::new(100)), &ctx, &mut Vec::new())
        })
    });
}

/// Per-step trace ON: pays one `state_hash()` and one Vec push per retired
/// instruction.
fn bench_run_until_yield_per_step_on(c: &mut Criterion) {
    let addi_word: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;
    let addi_bytes = addi_word.to_be_bytes();
    let mut mem = GuestMemory::new(4096);
    for i in 0..1000 {
        let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(i * 4), 4).unwrap();
        mem.apply_commit(range, &addi_bytes).unwrap();
    }

    c.bench_function("run_until_yield/per_step_on_addi100", |b| {
        b.iter(|| {
            let mut ppu = PpuExecutionUnit::new(UnitId::new(0));
            let ctx = ExecutionContext::new(&mem).with_trace_per_step(true);
            let r = ppu.run_until_yield(black_box(Budget::new(100)), &ctx, &mut Vec::new());
            // Drain per iteration; buffer would otherwise grow unbounded.
            let _ = ppu.drain_retired_state_hashes();
            r
        })
    });
}

/// Cost of one `PpuState::state_hash()` call in isolation.
fn bench_state_hash(c: &mut Criterion) {
    let mut s = PpuState::new();
    for (i, r) in s.gpr.iter_mut().enumerate() {
        *r = 0x1000 + i as u64;
    }
    s.lr = 0xdead_beef;
    s.ctr = 0xcafe_babe;
    s.xer = 1 << 29;
    s.cr = 0xa5a5_a5a5;
    c.bench_function("state_hash/full_register_file", |b| {
        b.iter(|| black_box(&s).state_hash())
    });
}

criterion_group!(
    run_benches,
    bench_run_until_yield_100,
    bench_run_until_yield_budget_1,
    bench_run_until_yield_mixed,
    bench_run_until_yield_per_step_off,
    bench_run_until_yield_per_step_on,
    bench_state_hash,
);

criterion_main!(decode_benches, execute_benches, run_benches);
