//! Core runtime microbenchmarks.
//!
//! Measures: commit_step with 0, 1, and 10 SharedWriteIntent effects.

#![allow(missing_docs)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};

use cellgov_core::commit::{CommitContext, CommitPipeline};
use cellgov_core::registry::UnitRegistry;
use cellgov_dma::{DmaQueue, FixedLatency};
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_exec::{ExecutionStepResult, LocalDiagnostics, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_sync::{MailboxRegistry, SignalRegistry};
use cellgov_time::{Budget, GuestTicks};

fn make_write_effect(addr: u64, data: &[u8]) -> Effect {
    Effect::SharedWriteIntent {
        range: ByteRange::new(GuestAddr::new(addr), data.len() as u64).unwrap(),
        bytes: WritePayload::new(data.to_vec()),
        ordering: PriorityClass::Normal,
        source: UnitId::new(0),
        source_time: GuestTicks::ZERO,
    }
}

fn make_step_result(effects: Vec<Effect>) -> ExecutionStepResult {
    ExecutionStepResult {
        yield_reason: YieldReason::BudgetExhausted,
        consumed_budget: Budget::new(1),
        emitted_effects: effects,
        local_diagnostics: LocalDiagnostics::with_pc(0x1000),
        fault: None,
        syscall_args: None,
    }
}

fn bench_commit_0_effects(c: &mut Criterion) {
    let mut pipeline = CommitPipeline::new();
    let result = make_step_result(vec![]);
    let latency = FixedLatency::new(10);

    c.bench_function("commit_step/0_effects", |b| {
        let mut mem = GuestMemory::new(4096);
        let mut units = UnitRegistry::new();
        let mut mailboxes = MailboxRegistry::new();
        let mut signals = SignalRegistry::new();
        let mut dma = DmaQueue::new();
        b.iter(|| {
            let mut ctx = CommitContext {
                memory: &mut mem,
                units: &mut units,
                mailboxes: &mut mailboxes,
                signals: &mut signals,
                dma_queue: &mut dma,
                dma_latency: &latency,
                now: GuestTicks::ZERO,
            };
            pipeline.process(black_box(&result), &mut ctx).unwrap()
        })
    });
}

fn bench_commit_1_effect(c: &mut Criterion) {
    let mut pipeline = CommitPipeline::new();
    let result = make_step_result(vec![make_write_effect(0x100, &[0xDE, 0xAD, 0xBE, 0xEF])]);
    let latency = FixedLatency::new(10);

    c.bench_function("commit_step/1_effect", |b| {
        let mut mem = GuestMemory::new(4096);
        let mut units = UnitRegistry::new();
        let mut mailboxes = MailboxRegistry::new();
        let mut signals = SignalRegistry::new();
        let mut dma = DmaQueue::new();
        b.iter(|| {
            let mut ctx = CommitContext {
                memory: &mut mem,
                units: &mut units,
                mailboxes: &mut mailboxes,
                signals: &mut signals,
                dma_queue: &mut dma,
                dma_latency: &latency,
                now: GuestTicks::ZERO,
            };
            pipeline.process(black_box(&result), &mut ctx).unwrap()
        })
    });
}

fn bench_commit_10_effects(c: &mut Criterion) {
    let mut pipeline = CommitPipeline::new();
    let effects: Vec<Effect> = (0..10)
        .map(|i| make_write_effect(i * 8, &[0xAB; 8]))
        .collect();
    let result = make_step_result(effects);
    let latency = FixedLatency::new(10);

    c.bench_function("commit_step/10_effects", |b| {
        let mut mem = GuestMemory::new(4096);
        let mut units = UnitRegistry::new();
        let mut mailboxes = MailboxRegistry::new();
        let mut signals = SignalRegistry::new();
        let mut dma = DmaQueue::new();
        b.iter(|| {
            let mut ctx = CommitContext {
                memory: &mut mem,
                units: &mut units,
                mailboxes: &mut mailboxes,
                signals: &mut signals,
                dma_queue: &mut dma,
                dma_latency: &latency,
                now: GuestTicks::ZERO,
            };
            pipeline.process(black_box(&result), &mut ctx).unwrap()
        })
    });
}

fn bench_commit_fault_discard(c: &mut Criterion) {
    let mut pipeline = CommitPipeline::new();
    // Fault step with 10 effects -- all should be discarded
    let effects: Vec<Effect> = (0..10)
        .map(|i| make_write_effect(i * 8, &[0xAB; 8]))
        .collect();
    let result = ExecutionStepResult {
        yield_reason: YieldReason::Fault,
        consumed_budget: Budget::new(1),
        emitted_effects: effects,
        local_diagnostics: LocalDiagnostics::with_pc(0x1000),
        fault: Some(cellgov_effects::FaultKind::Guest(0x0106_0000)),
        syscall_args: None,
    };
    let latency = FixedLatency::new(10);

    c.bench_function("commit_step/fault_discard_10", |b| {
        let mut mem = GuestMemory::new(4096);
        let mut units = UnitRegistry::new();
        let mut mailboxes = MailboxRegistry::new();
        let mut signals = SignalRegistry::new();
        let mut dma = DmaQueue::new();
        b.iter(|| {
            let mut ctx = CommitContext {
                memory: &mut mem,
                units: &mut units,
                mailboxes: &mut mailboxes,
                signals: &mut signals,
                dma_queue: &mut dma,
                dma_latency: &latency,
                now: GuestTicks::ZERO,
            };
            pipeline.process(black_box(&result), &mut ctx).unwrap()
        })
    });
}

criterion_group!(
    commit_benches,
    bench_commit_0_effects,
    bench_commit_1_effect,
    bench_commit_10_effects,
    bench_commit_fault_discard,
);

criterion_main!(commit_benches);
