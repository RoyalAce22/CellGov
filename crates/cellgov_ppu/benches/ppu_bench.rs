//! PPU microbenchmarks: decode, execute per-variant, and `run_until_yield`.

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
use cellgov_mem::GuestMemory;
use cellgov_ppu::decode::decode;
use cellgov_ppu::exec::execute;
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::shadow::PredecodedShadow;
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
                let _ = black_box(decode(black_box(w)));
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

// Budget=1: single-step mode, matching run-game's per-call pattern.
// Plain `iter()` keeps PPU construction + context inside the timed
// region because run-game pays that construction cost on every
// single-step iteration.
fn bench_run_until_yield_budget_1(c: &mut Criterion) {
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

/// Un-shadowed baseline for the addi100 fixture. Pairs with
/// `run_until_yield_shadowed/trace_off_addi100` to form the
/// quickening-guard delta. `iter_batched` keeps `PpuExecutionUnit::new`
/// out of the timed region.
fn bench_run_until_yield_per_step_off(c: &mut Criterion) {
    let pattern = [enc_li(3, 1)];
    let mem = fill_mem_with_pattern(&pattern);
    c.bench_function("run_until_yield/per_step_off_addi100", |b| {
        b.iter_batched(
            || PpuExecutionUnit::new(UnitId::new(0)),
            |mut ppu| {
                let ctx = ExecutionContext::new(&mem);
                let r = ppu.run_until_yield(black_box(Budget::new(100)), &ctx, &mut Vec::new());
                (ppu, r)
            },
            BatchSize::SmallInput,
        );
    });
}

/// Per-step trace ON, un-shadowed. The trace-cost delta against
/// `per_step_off_addi100` documents the miss-path trace overhead.
fn bench_run_until_yield_per_step_on(c: &mut Criterion) {
    let pattern = [enc_li(3, 1)];
    let mem = fill_mem_with_pattern(&pattern);
    c.bench_function("run_until_yield/per_step_on_addi100", |b| {
        b.iter_batched(
            || PpuExecutionUnit::new(UnitId::new(0)),
            |mut ppu| {
                let ctx = ExecutionContext::new(&mem).with_trace_per_step(true);
                let r = ppu.run_until_yield(black_box(Budget::new(100)), &ctx, &mut Vec::new());
                let drained = ppu.drain_retired_state_hashes();
                (ppu, r, drained)
            },
            BatchSize::SmallInput,
        );
    });
}

// Inline raw-instruction encoders. Mirror the `pub(super)` helpers
// in `shadow::test_support`; the bench crate has no access to
// module-private helpers.

// [PPC-Book1 p:51 s:3.3.8] addi: RT <- (RA|0) + EXTS(SI); RA=0 yields the
// `li RT,simm` extended mnemonic that the quickening pass rewrites to `Li`.
// [PPC-Book1 p:8 s:1.7.4 D-Form] OPCD(0:5)=14, RT(6:10), RA(11:15), SI(16:31).
fn enc_li(rt: u32, simm: i16) -> u32 {
    (14 << 26) | ((rt & 0x1F) << 21) | ((simm as u16) as u32)
}

// [PPC-Book1 p:42 s:3.3.3] stw: MEM(EA,4) <- (RS)32:63; EA = (RA|0)+EXTS(D).
// [PPC-Book1 p:8 s:1.7.4 D-Form] OPCD=36.
fn enc_stw(rs: u32, ra: u32, off: i16) -> u32 {
    (36 << 26) | ((rs & 0x1F) << 21) | ((ra & 0x1F) << 16) | (off as u16 as u32)
}

// [PPC-Book1 p:37 s:3.3.2] lwz: RT <- 32 zeros || MEM(EA,4); EA = (RA|0)+EXTS(D).
// [PPC-Book1 p:8 s:1.7.4 D-Form] OPCD=32.
fn enc_lwz(rt: u32, ra: u32, off: i16) -> u32 {
    (32 << 26) | ((rt & 0x1F) << 21) | ((ra & 0x1F) << 16) | (off as u16 as u32)
}

// [PPC-Book1 p:82 s:3.3.13] mflr = mfspr RT,LR (SPR=8). The 10-bit
// encoded SPR field is split with halves swapped relative to the SPR
// number: instruction bits 11:15 hold the LOW 5 bits of the SPR
// number, instruction bits 16:20 hold the HIGH 5 bits, so decode
// reassembles SPR# = (inst[16:20] << 5) | inst[11:15].
// SPR=8 = 0b00000_01000 -- low half 0b01000 = 8 at PPC bits 11:15,
// high half 0 at PPC bits 16:20. `(8 << 16)` writes that bits-11:15
// field (same instruction-bit position as RA in X-form).
// [PPC-Book1 p:9 s:1.7.8 XFX-Form] OPCD=31, XO(21:30)=339.
fn enc_mflr(rt: u32) -> u32 {
    (31 << 26) | ((rt & 0x1F) << 21) | (8 << 16) | (339 << 1)
}

// [PPC-Book1 p:81 s:3.3.13] mtlr = mtspr LR,RS (SPR=8); same XFX-form
// SPR-field half-swap as mflr above. XO(21:30)=467.
fn enc_mtlr(rs: u32) -> u32 {
    (31 << 26) | ((rs & 0x1F) << 21) | (8 << 16) | (467 << 1)
}

// [PPC-Book1 p:39 s:3.3.2] ld: RT <- MEM(EA,8); EA = (RA|0)+EXTS(DS||0b00).
// [PPC-Book1 p:8 s:1.7.5 DS-Form] OPCD=58, DS(16:29) || 0b00, XO(30:31)=00.
// `& 0xFFFC` clears the low 2 bits of the encoded offset (keeping DS aligned
// to 4 bytes) and the XO field (selecting plain ld, not ldu/lwa).
fn enc_ld(rt: u32, ra: u32, off: i16) -> u32 {
    (58 << 26) | ((rt & 0x1F) << 21) | ((ra & 0x1F) << 16) | ((off as u16 as u32) & 0xFFFC)
}

// [PPC-Book1 p:43 s:3.3.3] std: MEM(EA,8) <- (RS); EA = (RA|0)+EXTS(DS||0b00).
// [PPC-Book1 p:8 s:1.7.5 DS-Form] OPCD=62, XO(30:31)=00 selects std (not stdu).
fn enc_std(rs: u32, ra: u32, off: i16) -> u32 {
    (62 << 26) | ((rs & 0x1F) << 21) | ((ra & 0x1F) << 16) | ((off as u16 as u32) & 0xFFFC)
}

// [PPC-Book1 p:60 s:3.3.9] cmpi (cmpwi when L=0): signed compare of (RA)32:63
// against EXTS(SI); CR[BF] <- c || XER[SO]. The L bit at PPC bit 10 stays
// zero by virtue of no operand occupying that position.
// [PPC-Book1 p:8 s:1.7.4 D-Form] OPCD=11, BF(6:8), L(10), RA(11:15), SI(16:31).
fn enc_cmpwi(bf: u32, ra: u32, imm: i16) -> u32 {
    (11 << 26) | ((bf & 0x7) << 23) | ((ra & 0x1F) << 16) | (imm as u16 as u32)
}

// [PPC-Book1 p:60 s:3.3.9] cmp (cmpw when L=0): signed compare of (RA)32:63
// against (RB)32:63 into CR[BF].
// [PPC-Book1 p:9 s:1.7.6 X-Form] OPCD=31, XO(21:30)=0.
fn enc_cmpw(bf: u32, ra: u32, rb: u32) -> u32 {
    (31 << 26) | ((bf & 0x7) << 23) | ((ra & 0x1F) << 16) | ((rb & 0x1F) << 11)
}

// [PPC-Book1 p:24 s:2.4] bc BO,BI,target: branch B-form, non-linking with
// AA=LK=0. `& 0xFFFC` clears AA(30) and LK(31) regardless of caller bits and
// forces BD to 4-byte alignment; without it a non-aligned offset would
// silently flip AA/LK and produce a different branch class.
// [PPC-Book1 p:8 s:1.7.2 B-Form] OPCD=16, BO(6:10), BI(11:15), BD(16:29).
fn enc_bc(bo: u32, bi: u32, offset: i16) -> u32 {
    (16 << 26) | ((bo & 0x1F) << 21) | ((bi & 0x1F) << 16) | ((offset as u16 as u32) & 0xFFFC)
}

// 18-instruction tile hitting each of the nine super-pair fused
// variants exactly once: LiStw, MflrStw, LwzMtlr, MflrStd, LdMtlr,
// StdStd, LwzCmpwi, CmpwBc, CmpwiBc. Pair membership requirements
// per `shadow::superpair::make_super_pair`.
//
// Both bc forms use BO=12 BI=2 and comparisons set EQ=false, so every
// conditional branch falls through.
// [PPC-Book1 p:20 s:2.4.1 Figure 21] BO=12 (0b01100) is "branch if
// CR[BI]==1" with the BO_4 software hint clear.
// [PPC-Book1 p:18 s:2.3.1] CR0 bit assignments are LT(0), GT(1), EQ(2),
// SO(3); BI=2 indexes the EQ bit of CR0.
//
// r1 stays at 0 (PpuState::new zeros GPRs), so every load/store
// aliases the instruction bytes in mem and forwards through the
// store buffer.
fn mixed100_tile() -> [u32; 18] {
    [
        enc_li(3, 10),       // 0x00: addi r3, 0, 10 -- quickens to Li
        enc_stw(3, 1, 0),    // 0x04: LiStw       (li.rt == stw.rs == 3)
        enc_mflr(4),         // 0x08
        enc_stw(4, 1, 4),    // 0x0C: MflrStw     (mflr.rt == stw.rs == 4)
        enc_lwz(5, 1, 0),    // 0x10
        enc_mtlr(5),         // 0x14: LwzMtlr     (lwz.rt == mtlr.rs == 5)
        enc_mflr(6),         // 0x18
        enc_std(6, 1, 8),    // 0x1C: MflrStd     (mflr.rt == std.rs == 6)
        enc_ld(7, 1, 8),     // 0x20
        enc_mtlr(7),         // 0x24: LdMtlr      (ld.rt == mtlr.rs == 7)
        enc_std(3, 1, 16),   // 0x28
        enc_std(4, 1, 24),   // 0x2C: StdStd      (off2 - off1 == 8, ra1 == ra2)
        enc_lwz(8, 1, 32),   // 0x30
        enc_cmpwi(0, 8, 99), // 0x34: LwzCmpwi    (lwz.rt == cmpwi.ra == 8)
        enc_cmpw(0, 3, 9),   // 0x38              (r3 = 10, r9 = 0 -> EQ false)
        enc_bc(12, 2, 0),    // 0x3C: CmpwBc      (BO=12 BI=2 falls through)
        enc_cmpwi(0, 3, 99), // 0x40              (r3 = 10, imm = 99 -> EQ false)
        enc_bc(12, 2, 0),    // 0x44: CmpwiBc     (falls through)
    ]
}

// Populate a 4 KiB GuestMemory by repeating `pattern` (truncated to
// whole instructions). Far more memory than budget=100 can consume.
fn fill_mem_with_pattern(pattern: &[u32]) -> GuestMemory {
    const MEM_SIZE: usize = 4096;
    let slots = MEM_SIZE / 4;
    let mut mem = GuestMemory::new(MEM_SIZE);
    for i in 0..slots {
        let word = pattern[i % pattern.len()];
        let range =
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new((i * 4) as u64), 4).unwrap();
        mem.apply_commit(range, &word.to_be_bytes()).unwrap();
    }
    mem
}

// 1024 bytes covers the worst-case PC walk for budget=100 (400 bytes)
// with headroom. `shadow.clone()` runs in `iter_batched` setup, so
// shadow size does not pressure the timed region.
const SHADOW_SIZE: usize = 1024;

fn shadow_bytes_for(pattern: &[u32]) -> Vec<u8> {
    let slots = SHADOW_SIZE / 4;
    let mut bytes = Vec::with_capacity(SHADOW_SIZE);
    for i in 0..slots {
        bytes.extend_from_slice(&pattern[i % pattern.len()].to_be_bytes());
    }
    bytes
}

// Un-shadowed baseline for the mixed100 fixture.
fn bench_run_until_yield_per_step_off_mixed100(c: &mut Criterion) {
    let pattern = mixed100_tile();
    let mem = fill_mem_with_pattern(&pattern);
    c.bench_function("run_until_yield/per_step_off_mixed100", |b| {
        b.iter_batched(
            || PpuExecutionUnit::new(UnitId::new(0)),
            |mut ppu| {
                let ctx = ExecutionContext::new(&mem);
                let r = ppu.run_until_yield(black_box(Budget::new(100)), &ctx, &mut Vec::new());
                (ppu, r)
            },
            BatchSize::SmallInput,
        );
    });
}

// One-shot PC-bound probe shared by every shadowed arm. Runs once at
// registration and panics if PC walks past `SHADOW_SIZE`. Uses
// `assert!` because `cargo bench` runs the release profile.
fn assert_pc_bound(mem: &GuestMemory, shadow: &PredecodedShadow, trace_on: bool) {
    let mut probe = PpuExecutionUnit::new(UnitId::new(0));
    probe.set_instruction_shadow(shadow.clone());
    let ctx = if trace_on {
        ExecutionContext::new(mem).with_trace_per_step(true)
    } else {
        ExecutionContext::new(mem)
    };
    let _ = probe.run_until_yield(Budget::new(100), &ctx, &mut Vec::new());
    if trace_on {
        let _ = probe.drain_retired_state_hashes();
    }
    assert!(
        probe.state().pc < SHADOW_SIZE as u64,
        "PC walked out of shadow: {:#x} (SHADOW_SIZE = {})",
        probe.state().pc,
        SHADOW_SIZE,
    );
}

// Shadowed `run_until_yield`, trace OFF, addi100. Quickening-guard
// arm: homogeneous addi stream has no fusable pairs, so the delta
// vs `run_until_yield/per_step_off_addi100` isolates the quickening
// rule plus shadow-fast-path dispatch.
fn bench_run_until_yield_shadowed_off_addi100(c: &mut Criterion) {
    let pattern = [enc_li(3, 1)];
    let mem = fill_mem_with_pattern(&pattern);
    let shadow_bytes = shadow_bytes_for(&pattern);
    let shadow = PredecodedShadow::build(0, &shadow_bytes);
    assert!(
        matches!(shadow.get(0), Some(PpuInstruction::Li { .. })),
        "addi100 shadow slot 0 should quicken to Li; got {:?}",
        shadow.get(0)
    );
    assert_pc_bound(&mem, &shadow, false);
    c.bench_function("run_until_yield_shadowed/trace_off_addi100", |b| {
        b.iter_batched(
            || {
                let mut ppu = PpuExecutionUnit::new(UnitId::new(0));
                ppu.set_instruction_shadow(shadow.clone());
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

// Shadowed, trace OFF, mixed100. Super-pairing-guard arm: the delta
// vs `run_until_yield/per_step_off_mixed100` covers the nine fused
// variants the pair rule produces. `shadow.get(4)` is slot 1, the
// `Consumed` partner of the slot-0 fusion.
fn bench_run_until_yield_shadowed_off_mixed100(c: &mut Criterion) {
    let pattern = mixed100_tile();
    let mem = fill_mem_with_pattern(&pattern);
    let shadow_bytes = shadow_bytes_for(&pattern);
    let shadow = PredecodedShadow::build(0, &shadow_bytes);
    assert!(
        matches!(shadow.get(0), Some(PpuInstruction::LiStw { .. })),
        "mixed100 shadow slot 0 should fuse to LiStw; got {:?}",
        shadow.get(0)
    );
    assert!(
        matches!(shadow.get(4), Some(PpuInstruction::Consumed)),
        "mixed100 slot 1 (byte offset 4) should be Consumed; got {:?}",
        shadow.get(4)
    );
    assert_pc_bound(&mem, &shadow, false);
    c.bench_function("run_until_yield_shadowed/trace_off_mixed100", |b| {
        b.iter_batched(
            || {
                let mut ppu = PpuExecutionUnit::new(UnitId::new(0));
                ppu.set_instruction_shadow(shadow.clone());
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

// Shadowed, trace ON (per-step state-hash), addi100. Delta vs
// `trace_off_addi100` is the per-100-instruction trace cost on the
// shadowed fast path.
fn bench_run_until_yield_shadowed_hashes_addi100(c: &mut Criterion) {
    let pattern = [enc_li(3, 1)];
    let mem = fill_mem_with_pattern(&pattern);
    let shadow_bytes = shadow_bytes_for(&pattern);
    let shadow = PredecodedShadow::build(0, &shadow_bytes);
    assert!(
        matches!(shadow.get(0), Some(PpuInstruction::Li { .. })),
        "addi100 shadow slot 0 should quicken to Li; got {:?}",
        shadow.get(0)
    );
    assert_pc_bound(&mem, &shadow, true);
    c.bench_function("run_until_yield_shadowed/trace_hashes_addi100", |b| {
        b.iter_batched(
            || {
                let mut ppu = PpuExecutionUnit::new(UnitId::new(0));
                ppu.set_instruction_shadow(shadow.clone());
                ppu
            },
            |mut ppu| {
                let ctx = ExecutionContext::new(&mem).with_trace_per_step(true);
                let r = ppu.run_until_yield(black_box(Budget::new(100)), &ctx, &mut Vec::new());
                let drained = ppu.drain_retired_state_hashes();
                (ppu, r, drained)
            },
            BatchSize::SmallInput,
        );
    });
}

// Shadowed, trace ON, mixed100. Trace-cost delta on a mix that
// exercises every fused variant.
fn bench_run_until_yield_shadowed_hashes_mixed100(c: &mut Criterion) {
    let pattern = mixed100_tile();
    let mem = fill_mem_with_pattern(&pattern);
    let shadow_bytes = shadow_bytes_for(&pattern);
    let shadow = PredecodedShadow::build(0, &shadow_bytes);
    assert!(
        matches!(shadow.get(0), Some(PpuInstruction::LiStw { .. })),
        "mixed100 shadow slot 0 should fuse to LiStw; got {:?}",
        shadow.get(0)
    );
    assert!(
        matches!(shadow.get(4), Some(PpuInstruction::Consumed)),
        "mixed100 slot 1 (byte offset 4) should be Consumed; got {:?}",
        shadow.get(4)
    );
    assert_pc_bound(&mem, &shadow, true);
    c.bench_function("run_until_yield_shadowed/trace_hashes_mixed100", |b| {
        b.iter_batched(
            || {
                let mut ppu = PpuExecutionUnit::new(UnitId::new(0));
                ppu.set_instruction_shadow(shadow.clone());
                ppu
            },
            |mut ppu| {
                let ctx = ExecutionContext::new(&mem).with_trace_per_step(true);
                let r = ppu.run_until_yield(black_box(Budget::new(100)), &ctx, &mut Vec::new());
                let drained = ppu.drain_retired_state_hashes();
                (ppu, r, drained)
            },
            BatchSize::SmallInput,
        );
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
    bench_run_until_yield_budget_1,
    bench_run_until_yield_per_step_off,
    bench_run_until_yield_per_step_on,
    bench_run_until_yield_per_step_off_mixed100,
    bench_run_until_yield_shadowed_off_addi100,
    bench_run_until_yield_shadowed_off_mixed100,
    bench_run_until_yield_shadowed_hashes_addi100,
    bench_run_until_yield_shadowed_hashes_mixed100,
    bench_state_hash,
);

criterion_main!(decode_benches, execute_benches, run_benches);
