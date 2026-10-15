//! A PPU's access to a Raw SPU's problem-state window reaches the Raw
//! SPU's own state: the runtime routes a store into its local store or
//! a register. The Raw SPU's steps read that state and emit no effect
//! that names it, so the access conflicts with every step of the Raw SPU.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use std::cell::Cell;

use cellgov_core::Runtime;
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_explore::dependency::StepFootprint;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region, RegionAccess};
use cellgov_ps3_abi::hw::spu::SPU_IN_MBOX_OFFSET;
use cellgov_ps3_abi::lv2::spu::raw_window;
use cellgov_ps3_abi::lv2::syscall::{RAW_SPU_CREATE, SPU_INITIALIZE};
use cellgov_time::{Budget, GuestTicks, InstructionCost};

/// A syscall, or a word store.
#[derive(Clone, Copy)]
enum Op {
    Call([u64; 3]),
    Store(u64),
}

#[derive(Clone)]
struct Ppu {
    id: UnitId,
    ops: Vec<Op>,
    steps: Cell<usize>,
}

impl ExecutionUnit for Ppu {
    type Snapshot = usize;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= self.ops.len() {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let op = self.ops[self.steps.get()];
        self.steps.set(self.steps.get() + 1);
        let mut result = ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::with_pc(0x1000),
            fault: None,
            syscall_args: None,
        };
        match op {
            Op::Call(call) => {
                let mut args = [0u64; 9];
                args[..3].copy_from_slice(&call);
                result.yield_reason = YieldReason::Syscall;
                result.syscall_args = Some(args);
            }
            Op::Store(addr) => effects.push(Effect::shared_write(
                ByteRange::new(GuestAddr::new(addr), 4).unwrap(),
                WritePayload::from_slice(&[0, 0, 0, 1]),
                self.id,
                GuestTicks::ZERO,
            )),
        }
        result
    }

    fn snapshot(&self) -> usize {
        self.steps.get()
    }
}

/// The footprint of each of the PPU's steps, in order.
fn ppu_footprints(ops: Vec<Op>) -> (Vec<StepFootprint>, UnitId) {
    let memory = GuestMemory::from_regions(vec![
        Region::new(0, 0x1000, "main", PageSize::Page64K),
        Region::with_access(
            raw_window::BASE,
            raw_window::STRIDE as usize,
            "spu_reserved",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap();
    let mut rt = Runtime::new(memory, Budget::new(1), 100);
    // The Raw SPU's own unit is a stand-in; the footprint names it alone.
    rt.set_spu_factory(|id, _| Ok(Box::new(cellgov_testkit::world::CountingUnit::new(id, 50))));
    let count = ops.len();
    let ppu = rt.register_unit_with(|id| Ppu {
        id,
        ops,
        steps: Cell::new(0),
    });
    let mut footprints = Vec::new();
    while footprints.len() < count {
        let step = rt.step().unwrap();
        rt.commit_step(&step.result, &step.effects).unwrap();
        if step.unit == ppu {
            let mut fp =
                StepFootprint::from_step(step.unit, step.result.yield_reason, &step.effects);
            fp.note_commit(&rt, step.unit);
            footprints.push(fp);
        }
    }
    (footprints, rt.lv2_host().raw_spu_unit(0).unwrap())
}

#[test]
fn a_window_store_conflicts_with_every_step_of_the_raw_spu() {
    let (footprints, raw) = ppu_footprints(vec![
        Op::Call([SPU_INITIALIZE, 6, 1]),
        Op::Call([RAW_SPU_CREATE, 0x800, 0]),
        Op::Store(raw_window::BASE + 0x100),
        Op::Store(raw_window::BASE + raw_window::PROBLEM_STATE + u64::from(SPU_IN_MBOX_OFFSET)),
    ]);
    let raw_step = StepFootprint {
        local_store_owner: Some(raw),
        ..StepFootprint::default()
    };
    assert!(
        !footprints[0].conflicts(&raw_step),
        "a step that reaches no window is independent of the Raw SPU"
    );
    assert!(
        footprints[2].conflicts(&raw_step),
        "a local-store store through the window"
    );
    assert!(footprints[3].conflicts(&raw_step), "a register store");
}
