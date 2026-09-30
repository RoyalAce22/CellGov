//! An SPU stop reaches the runtime's trace and its stop-state query, and
//! the runtime resumes the unit at the next word.

use cellgov_core::Runtime;
use cellgov_exec::{RestartError, StopRegisters, UnitStatus, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;
use cellgov_trace::{TraceReader, TraceRecord};

/// `stop 0x102`: RR opcode 0 with the signal in bits 18:31.
const STOP_0X102: u32 = 0x0000_0102;
/// `il r3, 7`: RI16 opcode 0x081.
const IL_R3_7: u32 = (0x081 << 23) | (7 << 7) | 3;
/// `stop 0x2105`.
const STOP_0X2105: u32 = 0x0000_2105;

fn stopped_records(rt: &Runtime) -> Vec<TraceRecord> {
    TraceReader::new(rt.trace().bytes())
        .map(|record| record.expect("the runtime's own stream decodes"))
        .filter(|record| matches!(record, TraceRecord::UnitStopped { .. }))
        .collect()
}

fn step_once(rt: &mut Runtime) -> YieldReason {
    let step = rt.step().expect("a runnable unit");
    rt.commit_step(&step.result, &step.effects)
        .expect("the step commits");
    step.result.yield_reason
}

#[test]
fn a_stop_is_traced_readable_and_restartable_through_the_runtime() {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(100), 100);
    let unit = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        for (i, word) in [STOP_0X102, IL_R3_7, STOP_0X2105].iter().enumerate() {
            spu.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        spu
    });

    assert_eq!(step_once(&mut rt), YieldReason::Finished);
    let first = StopRegisters {
        status: 0x0102_0002,
        npc: 4,
    };
    assert_eq!(rt.unit_stop_registers(unit), Some(first));
    assert_eq!(
        stopped_records(&rt),
        vec![TraceRecord::UnitStopped {
            unit,
            status: first.status,
            npc: first.npc,
        }]
    );
    assert_eq!(
        rt.registry().effective_status(unit),
        Some(UnitStatus::Finished)
    );

    assert_eq!(rt.restart_unit(unit), Ok(()));
    assert_eq!(rt.unit_stop_registers(unit), None);
    assert_eq!(
        rt.registry().effective_status(unit),
        Some(UnitStatus::Runnable)
    );

    assert_eq!(step_once(&mut rt), YieldReason::Finished);
    let second = StopRegisters {
        status: 0x2105_0002,
        npc: 12,
    };
    assert_eq!(rt.unit_stop_registers(unit), Some(second));
    assert_eq!(stopped_records(&rt).len(), 2);
}

#[test]
fn a_unit_a_status_override_holds_refuses_a_restart() {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(100), 100);
    let unit = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        spu.state_mut().ls[..4].copy_from_slice(&STOP_0X102.to_be_bytes());
        spu
    });
    step_once(&mut rt);
    rt.set_unit_status_override(unit, UnitStatus::Finished);

    assert_eq!(rt.restart_unit(unit), Err(RestartError::Retired));
    assert!(rt.unit_stop_registers(unit).is_some());
}
