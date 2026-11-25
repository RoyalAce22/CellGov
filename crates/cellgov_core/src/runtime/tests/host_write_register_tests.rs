//! A register word takes a store only from the commit of a guest store:
//! `Runtime::host_write` refuses a write to it, and the commit of a guest
//! store runs the word's device rule.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionStepResult, LocalDiagnostics, YieldReason};
use cellgov_mem::{
    ByteRange, FaultContext, GuestAddr, GuestMemory, MemError, PageSize, Region, RegisterStore,
    RegisterWord,
};
use cellgov_time::{Budget, GuestTicks, InstructionCost};
use cellgov_trace::{HostWriter, TraceReader, TraceRecord};

use crate::commit::{CommitContext, CommitPipeline};

use super::*;

const SPAN: (u64, u64) = (0x240_0381_0000, 0x1_0000);
const WORD: u64 = SPAN.0 + 0x10;

/// The word in the state of its declaration.
const DECLARED: RegisterWord = RegisterWord {
    value: 0x10,
    known: 0xF0,
};

/// A rule that accepts every store and makes the stored word known.
fn take_value(words: &mut [RegisterWord], index: usize, store: RegisterStore) -> bool {
    words[index] = RegisterWord::known(store.value);
    true
}

/// Space 0 with a writable region and one register word in an unmodelled
/// span.
fn build() -> Runtime {
    let mut memory =
        GuestMemory::from_regions(vec![Region::new(0, 4096, "main", PageSize::Page64K)])
            .expect("one region");
    memory
        .declare_unmodelled(SPAN.0, SPAN.1)
        .expect("a span outside the region");
    memory
        .declare_registers(&[(WORD, DECLARED)], take_value)
        .expect("an aligned word of the span");
    Runtime::new(memory, Budget::new(4), 100)
}

fn word_range() -> ByteRange {
    ByteRange::new(GuestAddr::new(WORD), 4).expect("in-range test address")
}

fn host_writes(rt: &Runtime) -> usize {
    TraceReader::new(rt.trace().bytes())
        .map(|r| r.expect("the runtime's own stream decodes"))
        .filter(|r| matches!(r, TraceRecord::HostWrite { .. }))
        .count()
}

#[test]
fn a_four_byte_host_write_to_a_register_word_is_refused_and_keeps_the_device_state() {
    let mut rt = build();
    let hash = rt.memory.content_hash();
    let written = rt.host_write(
        HostWriter::Lv2Effect,
        AddressSpaceId::BOOT,
        word_range(),
        &[0, 0, 0, 2],
        None,
    );
    assert!(
        matches!(
            written,
            Err(MemError::Unmapped(FaultContext { addr, .. })) if addr == WORD
        ),
        "{written:?}"
    );
    assert_eq!(rt.memory.register_word(WORD), Some(DECLARED));
    assert_eq!(rt.memory.content_hash(), hash);
    assert_eq!(host_writes(&rt), 0);
}

#[test]
fn a_guest_store_to_a_register_word_runs_the_device_rule_through_the_commit() {
    let mut rt = build();
    let effects = vec![Effect::shared_write(
        word_range(),
        WritePayload::new(vec![0, 0, 0, 2]),
        UnitId::new(0),
        GuestTicks::ZERO,
    )];
    let result = ExecutionStepResult {
        yield_reason: YieldReason::Finished,
        consumed_cost: InstructionCost::new(1),
        local_diagnostics: LocalDiagnostics::empty(),
        fault: None,
        syscall_args: None,
    };
    let mut rsx_label_writes = 0u64;
    let mut ctx = CommitContext {
        space: 0,
        memory: &mut rt.memory,
        units: &mut rt.registry,
        mailboxes: &mut rt.mailbox_registry,
        signals: &mut rt.signal_registry,
        dma_queue: &mut rt.dma_queue,
        dma_latency: rt.dma_latency.as_ref(),
        now: GuestTicks::ZERO,
        reservations: &mut rt.reservations,
        rsx_label_base: 0,
        rsx_flip: &mut rt.rsx_flip,
        rsx_label_writes_committed: &mut rsx_label_writes,
        tap: None,
    };
    let outcome = CommitPipeline::new()
        .process(&result, &effects, &mut ctx)
        .expect("the device accepts the store");
    assert_eq!(outcome.writes_committed, 1);
    assert_eq!(rt.memory.register_word(WORD), Some(RegisterWord::known(2)));
}
