//! A PPU writes an SPU thread's signal-notification registers through
//! `sys_spu_thread_write_snr`, in the modes `sys_spu_thread_set_spu_cfg`
//! gave the thread, and the SPU reads what those modes leave.

// [CBEA p:239 s:16.4] each signal-notification register either overwrites its contents or ORs the data written into them.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use std::cell::Cell;

use cellgov_core::Runtime;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_lv2::thread_group::MAX_SLOTS_PER_GROUP;
use cellgov_lv2::GroupState;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{SPU_RD_SIG_NOTIFY_1, SPU_RD_SIG_NOTIFY_2};
use cellgov_ps3_abi::lv2::syscall::{SPU_THREAD_SET_SPU_CFG, SPU_THREAD_WRITE_SNR};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::{Budget, InstructionCost};

/// Issues one syscall per step from a fixed list, then finishes.
#[derive(Clone)]
struct Caller {
    id: UnitId,
    calls: Vec<[u64; 4]>,
    steps: Cell<usize>,
}

impl ExecutionUnit for Caller {
    type Snapshot = usize;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= self.calls.len() {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let call = self.calls[self.steps.get()];
        self.steps.set(self.steps.get() + 1);
        let mut args = [0u64; 9];
        args[..4].copy_from_slice(&call);
        ExecutionStepResult {
            yield_reason: YieldReason::Syscall,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::with_pc(0x1000),
            fault: None,
            syscall_args: Some(args),
        }
    }

    fn snapshot(&self) -> usize {
        self.steps.get()
    }
}

/// `rdch rt, $ch<channel>`.
fn rdch(channel: u8, rt: u32) -> u32 {
    (0x00D << 21) | (u32::from(channel) << 7) | rt
}

/// `il $20, 0`, which touches nothing the test reads.
const FILLER: u32 = (0x081 << 23) | 20;

/// The SPU's registers 3 and 4 after the caller configures the thread
/// with `config`, writes 0x10 then 0x20 to register 1 and 0x01 then 0x02
/// to register 2, and the SPU reads both.
fn read_after_writes(config: u64) -> (u32, u32) {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(1), 200);
    // The fillers keep the SPU's reads behind the caller's five calls.
    let mut program = vec![FILLER; 8];
    program.extend([
        rdch(SPU_RD_SIG_NOTIFY_1, 3),
        rdch(SPU_RD_SIG_NOTIFY_2, 4),
        0,
    ]);
    let groups = rt.lv2_host_mut().thread_groups_mut();
    let group = groups.create(1).unwrap();
    groups
        .initialize_thread(
            group,
            0,
            cellgov_lv2::SpuImageHandle::new(1).unwrap(),
            [0; 4],
        )
        .unwrap();
    let thread = u64::from(group * MAX_SLOTS_PER_GROUP);
    let caller = rt.register_unit_with(|id| Caller {
        id,
        calls: vec![
            [SPU_THREAD_SET_SPU_CFG, thread, config, 0],
            [SPU_THREAD_WRITE_SNR, thread, 0, 0x10],
            [SPU_THREAD_WRITE_SNR, thread, 0, 0x20],
            [SPU_THREAD_WRITE_SNR, thread, 1, 0x01],
            [SPU_THREAD_WRITE_SNR, thread, 1, 0x02],
        ],
        steps: Cell::new(0),
    });
    let spu = rt.register_unit_with(|id| {
        let mut unit = SpuExecutionUnit::new(id);
        for (i, word) in program.iter().enumerate() {
            unit.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        unit
    });
    let groups = rt.lv2_host_mut().thread_groups_mut();
    groups.get_mut(group).unwrap().state = GroupState::Running;
    groups.record_spu(spu, group, 0).unwrap();

    let mut finished = 0;
    for _ in 0..200 {
        let Ok(step) = rt.step() else { break };
        finished += usize::from(step.result.yield_reason == YieldReason::Finished);
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
        if step.unit == caller {
            assert!(
                rt.last_lv2_effects()
                    .iter()
                    .all(|e| matches!(e, Effect::SpuSignalWrite { .. })),
                "{:?}",
                rt.last_lv2_effects()
            );
        }
        if rt.registry().effective_status(spu) == Some(UnitStatus::Finished) {
            break;
        }
    }
    assert!(finished >= 1, "the SPU stops");
    let state = rt
        .registry()
        .get(spu)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .unwrap()
        .state()
        .clone();
    (state.reg_word(3), state.reg_word(4))
}

#[test]
fn the_configuration_sets_each_registers_mode_for_the_writes() {
    assert_eq!(
        read_after_writes(0b10),
        (0x20, 0x03),
        "register 1 overwrites, register 2 ORs"
    );
    assert_eq!(
        read_after_writes(0b01),
        (0x30, 0x02),
        "register 1 ORs, register 2 overwrites"
    );
    assert_eq!(read_after_writes(0), (0x20, 0x02), "both overwrite");
}
