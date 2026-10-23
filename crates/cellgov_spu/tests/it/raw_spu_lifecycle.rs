//! A PPU creates and destroys a Raw SPU through `sys_raw_spu_create`
//! and `sys_raw_spu_destroy`: the create registers a stopped SPU unit,
//! binds it to the id it writes back, and gives it an inbound mailbox;
//! the destroy retires the unit.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use std::cell::Cell;

use cellgov_core::Runtime;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, ProblemStateError,
    UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::syscall::{RAW_SPU_CREATE, RAW_SPU_DESTROY, SPU_INITIALIZE};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::{Budget, InstructionCost};

/// Issues one syscall per step from a fixed list, then finishes.
#[derive(Clone)]
struct Caller {
    id: UnitId,
    calls: Vec<[u64; 3]>,
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
        args[..3].copy_from_slice(&call);
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

const ID_PTR: u64 = 0x800;

/// Run the caller's next call to its commit; return every unit that
/// stepped on the way.
fn run_call(rt: &mut Runtime, caller: UnitId) -> Vec<UnitId> {
    let mut stepped = Vec::new();
    for _ in 0..20 {
        let step = rt.step().expect("a unit steps");
        rt.commit_unchecked(&step.result, &step.effects)
            .expect("the step commits");
        stepped.push(step.unit);
        if step.unit == caller {
            return stepped;
        }
    }
    panic!("the caller never stepped: {stepped:?}");
}

#[test]
fn a_raw_spu_is_a_stopped_spu_unit_until_its_destroy_retires_it() {
    let mut memory = GuestMemory::new(0x1000);
    memory
        .apply_commit(
            ByteRange::new(GuestAddr::new(ID_PTR), 4).unwrap(),
            &[0xEE; 4],
        )
        .unwrap();
    let mut rt = Runtime::new(memory, Budget::new(1), 200);
    rt.set_spu_factory(|id, init| {
        assert!(matches!(&init.image, cellgov_lv2::SpuLoadImage::Segments(s) if s.is_empty()));
        Ok(Box::new(SpuExecutionUnit::new(id)))
    });
    let caller = rt.register_unit_with(|id| Caller {
        id,
        calls: vec![
            [SPU_INITIALIZE, 6, 1],
            [RAW_SPU_CREATE, ID_PTR, 0],
            [RAW_SPU_DESTROY, 0, 0],
        ],
        steps: Cell::new(0),
    });

    run_call(&mut rt, caller);
    assert_eq!(rt.lv2_host().raw_spu_unit(0), None);
    run_call(&mut rt, caller);
    let raw = rt.lv2_host().raw_spu_unit(0).expect("Raw SPU 0 exists");
    let id = rt
        .memory()
        .read(ByteRange::new(GuestAddr::new(ID_PTR), 4).unwrap())
        .unwrap();
    assert_eq!(id, [0, 0, 0, 0], "the create writes id 0");
    assert_eq!(
        rt.registry().effective_status(raw),
        Some(UnitStatus::Finished),
        "a Raw SPU waits, stopped, for its run control"
    );
    assert!(
        rt.unit_stop_registers(raw).is_some(),
        "the SPU holds a stop"
    );
    rt.write_unit_in_mbox(raw, 0x41)
        .expect("the Raw SPU has an inbound mailbox");

    let stepped = run_call(&mut rt, caller);
    assert!(!stepped.contains(&raw), "a stopped Raw SPU never steps");
    assert_eq!(rt.lv2_host().raw_spu_unit(0), None);
    assert_eq!(
        rt.write_unit_in_mbox(raw, 0x41),
        Err(ProblemStateError::Retired),
        "the destroy retires the unit"
    );
}
