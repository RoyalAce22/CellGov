//! A PPU writes and reads an SPU thread's local store through
//! `sys_spu_thread_write_ls` and `sys_spu_thread_read_ls`: the write
//! lands in the SPU's local store, and a read returns the bytes there,
//! zero-extended, in guest memory.

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
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::syscall::{SPU_THREAD_READ_LS, SPU_THREAD_WRITE_LS};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::{Budget, InstructionCost};

/// Issues one syscall per step from a fixed list, then finishes.
#[derive(Clone)]
struct Caller {
    id: UnitId,
    calls: Vec<[u64; 5]>,
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
        args[..5].copy_from_slice(&call);
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

/// `il $20, 0`, which touches nothing the test reads.
const FILLER: u32 = (0x081 << 23) | 20;
const DATA_LSA: usize = 0x2000;
const WRITTEN_PTR: u64 = 0x800;
const UNTOUCHED_PTR: u64 = 0x808;

#[test]
fn a_write_lands_in_local_store_and_a_read_returns_it_zero_extended() {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(1), 200);
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
    let lsa = DATA_LSA as u64;
    let caller = rt.register_unit_with(|id| Caller {
        id,
        calls: vec![
            [SPU_THREAD_WRITE_LS, thread, lsa, 0xCAFE_BABE, 4],
            [SPU_THREAD_READ_LS, thread, lsa, WRITTEN_PTR, 4],
            [SPU_THREAD_READ_LS, thread, lsa + 4, UNTOUCHED_PTR, 4],
        ],
        steps: Cell::new(0),
    });
    let spu = rt.register_unit_with(|id| {
        let mut unit = SpuExecutionUnit::new(id);
        // The fillers keep the SPU running behind the caller's calls.
        for i in 0..16 {
            unit.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&FILLER.to_be_bytes());
        }
        unit.state_mut().ls[DATA_LSA + 4..DATA_LSA + 8].copy_from_slice(&[0x11, 0x22, 0x33, 0x44]);
        unit
    });
    let groups = rt.lv2_host_mut().thread_groups_mut();
    groups.get_mut(group).unwrap().state = GroupState::Running;
    groups.record_spu(spu, group, 0).unwrap();

    let mut calls_done = 0;
    for _ in 0..200 {
        let Ok(mut step) = rt.step() else { break };
        rt.commit_step(&mut step).expect("the step commits");
        if step.unit == caller {
            calls_done += 1;
            assert!(
                rt.observations().lv2_effects.iter().all(|e| matches!(
                    e,
                    Effect::SpuLocalStoreWrite { .. }
                        | Effect::SpuLocalStoreRead { .. }
                        | Effect::SharedWriteIntent { .. }
                )),
                "{:?}",
                rt.observations().lv2_effects
            );
        }
        if calls_done == 3 {
            break;
        }
    }
    assert_eq!(calls_done, 3, "the caller made every call");

    let state = rt
        .registry()
        .get(spu)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .unwrap()
        .state()
        .clone();
    assert_eq!(state.ls[DATA_LSA..DATA_LSA + 4], [0xCA, 0xFE, 0xBA, 0xBE]);
    let word = |at: u64| {
        let bytes = rt
            .memory()
            .read(ByteRange::new(GuestAddr::new(at), 8).unwrap())
            .unwrap();
        u64::from_be_bytes(bytes.try_into().unwrap())
    };
    assert_eq!(word(WRITTEN_PTR), 0xCAFE_BABE);
    assert_eq!(word(UNTOUCHED_PTR), 0x1122_3344);
}
