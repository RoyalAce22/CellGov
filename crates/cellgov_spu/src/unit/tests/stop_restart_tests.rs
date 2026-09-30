//! A stop leaves the unit restartable at the next word, and a restart
//! resumes it there.

use crate::stop::{SpuStop, SpuStopKind};
use crate::SpuExecutionUnit;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionUnit, RestartError, StopRegisters, UnitStatus, YieldReason,
};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

/// `stop 0x102`: RR opcode 0 with the signal in bits 18:31.
const STOP_0X102: u32 = 0x0000_0102;
/// `stopd`: RR opcode 0x140.
const STOPD: u32 = 0x140 << 21;
/// `il r3, 7`: RI16 opcode 0x081.
const IL_R3_7: u32 = (0x081 << 23) | (7 << 7) | 3;

fn unit_with(words: &[u32]) -> SpuExecutionUnit {
    let mut unit = SpuExecutionUnit::new(UnitId::new(5));
    for (i, word) in words.iter().enumerate() {
        unit.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    unit
}

fn run(unit: &mut SpuExecutionUnit) -> YieldReason {
    let mem = GuestMemory::new(0x1000);
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    unit.run_until_yield(Budget::new(100), &ctx, &mut effects)
        .yield_reason
}

// [SPU-ISA p:238 s:10] stop: PC <- PC + 4 & LSLR; [CBEA p:95 s:8.5.3] SPU_NPC holds the address the SPU resumes at.
#[test]
fn a_stop_records_its_code_and_the_next_word() {
    let mut unit = unit_with(&[STOP_0X102, IL_R3_7, STOP_0X102]);
    assert_eq!(run(&mut unit), YieldReason::Finished);
    assert_eq!(unit.status(), UnitStatus::Finished);
    let expected = SpuStop {
        kind: SpuStopKind::Stop,
        code: 0x102,
        npc: 4,
    };
    assert_eq!(unit.state().stop, Some(expected));
    assert_eq!(unit.state().pc, 4);
    assert_eq!(unit.snapshot().stop, Some(expected));
    assert_eq!(
        unit.stop_registers(),
        Some(StopRegisters {
            status: 0x0102_0002,
            npc: 4
        })
    );
}

// [CBEA p:95 s:8.5.3] a restart resumes at SPU_NPC; [CBEA p:94 s:8.5.2] it clears the stop status.
#[test]
fn a_restart_resumes_at_the_next_word() {
    let mut unit = unit_with(&[STOP_0X102, IL_R3_7, STOP_0X102]);
    run(&mut unit);
    assert_eq!(unit.restart(), Ok(()));
    assert_eq!(unit.status(), UnitStatus::Runnable);
    assert_eq!(unit.state().stop, None);
    assert_eq!(unit.stop_registers(), None);

    assert_eq!(run(&mut unit), YieldReason::Finished);
    assert_eq!(unit.state().reg_word(3), 7);
    assert_eq!(unit.state().stop.map(|stop| stop.npc), Some(12));
}

#[test]
fn a_unit_that_did_not_stop_refuses_a_restart() {
    let mut unit = unit_with(&[IL_R3_7]);
    assert_eq!(unit.restart(), Err(RestartError::NotStopped));
    assert_eq!(unit.status(), UnitStatus::Runnable);
}

// [CBEA p:93 s:8.5.2] a stopd always sets StopCode to x'3FFF'.
#[test]
fn a_stopd_reports_the_breakpoint_code() {
    let mut unit = unit_with(&[STOPD]);
    assert_eq!(run(&mut unit), YieldReason::Finished);
    assert_eq!(
        unit.stop_registers(),
        Some(StopRegisters {
            status: 0x3FFF_0002,
            npc: 4
        })
    );
}
