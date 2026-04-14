//! PPU microbenchmarks.
//!
//! Measures: decode throughput, execute per-variant latency,
//! run_until_yield with Budget=100 on a synthetic instruction stream.

#![allow(missing_docs)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};

use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::GuestMemory;
use cellgov_ppu::decode::decode;
use cellgov_ppu::exec::execute;
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::state::PpuState;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::Budget;

// --- Decode throughput ---

fn bench_decode_addi(c: &mut Criterion) {
    // addi r3, r0, 1 => opcode 14, rt=3, ra=0, imm=1
    let raw: u32 = (14 << 26) | (3 << 21) | 1;
    c.bench_function("decode/addi", |b| b.iter(|| decode(black_box(raw))));
}

fn bench_decode_lwz(c: &mut Criterion) {
    // lwz r3, 0(r1) => opcode 32, rt=3, ra=1, imm=0
    let raw: u32 = (32 << 26) | (3 << 21) | (1 << 16);
    c.bench_function("decode/lwz", |b| b.iter(|| decode(black_box(raw))));
}

fn bench_decode_stw(c: &mut Criterion) {
    // stw r3, 0(r1) => opcode 36, rs=3, ra=1, imm=0
    let raw: u32 = (36 << 26) | (3 << 21) | (1 << 16);
    c.bench_function("decode/stw", |b| b.iter(|| decode(black_box(raw))));
}

fn bench_decode_bc(c: &mut Criterion) {
    // bc 12, 2, +8 => opcode 16, BO=12, BI=2, BD=8
    let raw: u32 = (16 << 26) | (12 << 21) | (2 << 16) | 8;
    c.bench_function("decode/bc", |b| b.iter(|| decode(black_box(raw))));
}

fn bench_decode_xo_add(c: &mut Criterion) {
    // add r3, r4, r5 => opcode 31, XO=266, rt=3, ra=4, rb=5
    let raw: u32 = (31 << 26) | (3 << 21) | (4 << 16) | (5 << 11) | (266 << 1);
    c.bench_function("decode/add_xo", |b| b.iter(|| decode(black_box(raw))));
}

fn bench_decode_mixed_batch(c: &mut Criterion) {
    // Batch of 8 different instruction types to measure average throughput
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

// --- Execute per-variant latency ---

fn bench_execute_addi(c: &mut Criterion) {
    let insn = PpuInstruction::Addi {
        rt: 3,
        ra: 0,
        imm: 42,
    };
    let uid = UnitId::new(0);
    c.bench_function("execute/addi", |b| {
        let mut state = PpuState::new();
        b.iter(|| {
            execute(black_box(&insn), &mut state, uid);
        })
    });
}

fn bench_execute_add(c: &mut Criterion) {
    let insn = PpuInstruction::Add {
        rt: 3,
        ra: 4,
        rb: 5,
    };
    let uid = UnitId::new(0);
    c.bench_function("execute/add", |b| {
        let mut state = PpuState::new();
        state.gpr[4] = 100;
        state.gpr[5] = 200;
        b.iter(|| {
            execute(black_box(&insn), &mut state, uid);
        })
    });
}

fn bench_execute_lwz(c: &mut Criterion) {
    // Execute returns Load outcome -- no memory access in execute()
    let insn = PpuInstruction::Lwz {
        rt: 3,
        ra: 1,
        imm: 0,
    };
    let uid = UnitId::new(0);
    c.bench_function("execute/lwz", |b| {
        let mut state = PpuState::new();
        state.gpr[1] = 0x1000;
        b.iter(|| {
            execute(black_box(&insn), &mut state, uid);
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
        b.iter(|| {
            execute(black_box(&insn), &mut state, uid);
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
        b.iter(|| {
            execute(black_box(&insn), &mut state, uid);
        })
    });
}

fn bench_execute_b(c: &mut Criterion) {
    let insn = PpuInstruction::B {
        offset: 0x800,
        link: false,
    };
    let uid = UnitId::new(0);
    c.bench_function("execute/b", |b| {
        let mut state = PpuState::new();
        state.pc = 0x800;
        b.iter(|| {
            state.pc = 0x800;
            execute(black_box(&insn), &mut state, uid);
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
    };
    let uid = UnitId::new(0);
    c.bench_function("execute/rlwinm", |b| {
        let mut state = PpuState::new();
        state.gpr[4] = 0x12345678;
        b.iter(|| {
            execute(black_box(&insn), &mut state, uid);
        })
    });
}

// --- run_until_yield with Budget=100 ---

fn bench_run_until_yield_100(c: &mut Criterion) {
    // Fill memory with a tight loop of `addi r3, r3, 1` instructions.
    // addi r3, r3, 1 => opcode 14, rt=3, ra=3, imm=1
    let addi_word: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;
    let addi_bytes = addi_word.to_be_bytes();

    let mem_size = 4096;
    let mut mem = GuestMemory::new(mem_size);
    // Write 1000 addi instructions starting at address 0
    for i in 0..1000 {
        let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(i * 4), 4).unwrap();
        mem.apply_commit(range, &addi_bytes).unwrap();
    }

    c.bench_function("run_until_yield/budget_100_addi", |b| {
        b.iter(|| {
            let mut ppu = PpuExecutionUnit::new(UnitId::new(0));
            let ctx = ExecutionContext::new(&mem);
            ppu.run_until_yield(black_box(Budget::new(100)), &ctx)
        })
    });
}

fn bench_run_until_yield_budget_1(c: &mut Criterion) {
    // Single-step mode (Budget=1), matching run-game usage.
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
            ppu.run_until_yield(black_box(Budget::new(1)), &ctx)
        })
    });
}

fn bench_run_until_yield_mixed(c: &mut Criterion) {
    // A mix of instructions: addi, add, cmpwi, b (loop back).
    // Simulates a realistic tight loop.
    let mut mem = GuestMemory::new(4096);
    let instructions: &[(u64, u32)] = &[
        // 0x00: addi r3, r3, 1
        (0x00, (14 << 26) | (3 << 21) | (3 << 16) | 1),
        // 0x04: add r4, r3, r5
        (
            0x04,
            (31 << 26) | (4 << 21) | (3 << 16) | (5 << 11) | (266 << 1),
        ),
        // 0x08: cmpwi cr0, r3, 100
        (0x08, (11 << 26) | (3 << 16) | 100),
        // 0x0C: addi r5, r5, 1 -- straight-line so the bench runs to budget
        // exhaustion without an infinite loop.
        (0x0C, (14 << 26) | (5 << 21) | (5 << 16) | 1),
    ];
    // Tile this 4-instruction block across memory
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
            ppu.run_until_yield(black_box(Budget::new(100)), &ctx)
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

// --- per-step state-hash trace cost ---

/// Run the same 100-instruction addi loop with per-step state-hash
/// tracing OFF. Pairs with `bench_run_until_yield_per_step_on` to
/// quantify the cost of leaving the per-step path enabled. The OFF
/// branch is the zero-cost path the runtime defaults to.
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
            // Default: per_step_trace == false.
            let ctx = ExecutionContext::new(&mem);
            ppu.run_until_yield(black_box(Budget::new(100)), &ctx)
        })
    });
}

/// Same workload with per-step tracing ON. This pays the
/// `state_hash()` call plus a Vec push every retired instruction.
/// The runtime opts in only when the dev explicitly requests
/// per-step trace.
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
            ppu.set_per_step_trace(true);
            let ctx = ExecutionContext::new(&mem);
            let r = ppu.run_until_yield(black_box(Budget::new(100)), &ctx);
            // Drain every iteration so the buffer does not grow
            // unbounded across criterion samples.
            let _ = ppu.drain_retired_state_hashes();
            r
        })
    });
}

// --- per-call cost of PpuState::state_hash() ---

/// Cost of one PpuState::state_hash() call. Pulled out of the
/// per-step emission bench so a regression in the hash function
/// itself can be distinguished from a regression in the surrounding
/// emission path. Hash input is 324 bytes (32 GPRs + LR + CTR + XER
/// + CR).
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
