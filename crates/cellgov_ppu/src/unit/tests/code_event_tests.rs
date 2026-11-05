//! A traced batch records each retired fetch and each step of the
//! instruction-modification sequence, in program order.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{CodeEvent, ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

const STW_R3_128: u32 = (36 << 26) | (3 << 21) | 128;
const STB_R3_132: u32 = (38 << 26) | (3 << 21) | 132;
const DCBST_R6: u32 = 0x7C00_306C;
const DCBF_R6: u32 = 0x7C00_30AC;
const SYNC: u32 = 0x7C00_04AC;
const LWSYNC: u32 = 0x7C20_04AC;
const ICBI_R6: u32 = 0x7C00_37AC;
const ISYNC: u32 = 0x4C00_012C;
const NOP: u32 = 0x6000_0000;
const SC: u32 = 0x4400_0002;

/// Runs `program` from address 0 with r3 = 0x1234, r5 outside guest
/// memory and r6 = 0x80, and returns how the batch yielded and the
/// events it left.
fn run(program: &[u32], per_step: bool) -> (YieldReason, Vec<CodeEvent>) {
    let mut mem = GuestMemory::new(256);
    for (i, &word) in program.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(4 * i as u64), 4).unwrap();
        mem.apply_commit(range, &word.to_be_bytes()).unwrap();
    }
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(3, 0x1234);
    unit.state_mut().set_gpr(5, 0xFFFF_0000);
    unit.state_mut().set_gpr(6, 0x80);
    let ctx = ExecutionContext::new(&mem).with_trace_per_step(per_step);
    let mut effects = Vec::new();
    let budget = Budget::new(program.len() as u64);
    let result = unit.run_until_yield(budget, &ctx, &mut effects);
    (result.yield_reason, unit.drain_code_events())
}

#[test]
fn the_events_follow_program_order_and_a_fetch_records_once_per_line_run() {
    let program = [
        STW_R3_128, STB_R3_132, DCBST_R6, SYNC, ICBI_R6, ISYNC, LWSYNC, DCBF_R6, SC,
    ];
    let (yielded, events) = run(&program, true);

    assert_eq!(yielded, YieldReason::Syscall);
    let fetch = |pc| CodeEvent::Fetch { pc };
    assert_eq!(
        events,
        [
            fetch(0),
            CodeEvent::Store {
                addr: 128,
                len: 4,
                word: Some(0x1234)
            },
            fetch(4),
            CodeEvent::Store {
                addr: 132,
                len: 1,
                word: None
            },
            fetch(8),
            CodeEvent::Dcbst { addr: 0x80 },
            fetch(12),
            CodeEvent::Sync,
            fetch(16),
            CodeEvent::Icbi { addr: 0x80 },
            fetch(20),
            CodeEvent::Isync,
            // `lwsync` is no step of the sequence, so the fetch of the
            // `dcbf` after it continues the run from 24.
            fetch(24),
            CodeEvent::Dcbst { addr: 0x80 },
            fetch(32),
            CodeEvent::ContextSync,
        ]
    );
}

#[test]
fn a_batch_without_per_step_tracing_records_no_event() {
    let (_, events) = run(&[STW_R3_128, DCBST_R6, SYNC, ICBI_R6, ISYNC, SC], false);
    assert_eq!(events, []);
}

#[test]
fn a_faulting_instruction_records_no_event_and_a_faulted_batch_keeps_none() {
    let lwz_r6_r5 = (32 << 26) | (6 << 21) | (5 << 16);
    let (yielded, events) = run(&[NOP, lwz_r6_r5], true);
    assert_eq!(yielded, YieldReason::BudgetExhausted, "the nop retires");
    assert_eq!(events, [CodeEvent::Fetch { pc: 0 }]);

    let (yielded, events) = run(&[lwz_r6_r5], true);
    assert_eq!(yielded, YieldReason::Fault);
    assert_eq!(events, []);
}
