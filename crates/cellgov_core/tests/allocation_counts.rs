//! Allocation bounds for steady-state runtime steps.

#![allow(unsafe_code, reason = "a global allocator is the test subject")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use cellgov_core::{Runtime, RuntimeMode};
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, GuestTicks, InstructionCost};

struct CountingAllocator;

static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const STEPS: usize = 64;
const TRIVIAL_STEP_ALLOCATIONS: usize = 0;
// One staging vector and two commit-side range vectors grow on a
// single write. The payload itself is inline and adds no allocation.
const WRITE_STEP_ALLOCATIONS: usize = 3;

#[derive(Clone)]
struct Unit {
    id: UnitId,
    writes: bool,
}

impl ExecutionUnit for Unit {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        UnitStatus::Runnable
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        if self.writes {
            effects.push(Effect::shared_write(
                ByteRange::new(GuestAddr::new(0), 4).expect("four-byte range"),
                WritePayload::from_slice(&[1, 2, 3, 4]),
                self.id,
                GuestTicks::ZERO,
            ));
        }
        ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) {}
}

fn allocations_after_warmup(writes: bool) -> usize {
    let mut runtime = Runtime::new(GuestMemory::new(64), Budget::new(1), 1_000);
    runtime.set_mode(RuntimeMode::FaultDriven);
    runtime.register_unit_with(|id| Unit { id, writes });
    let mut warmup = runtime.step().expect("warmup step");
    runtime
        .commit_step_and_recycle(&mut warmup)
        .expect("warmup commit");

    ALLOCATIONS.store(0, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);
    for _ in 0..STEPS {
        let mut step = runtime.step().expect("steady step");
        runtime
            .commit_step_and_recycle(&mut step)
            .expect("steady commit");
    }
    COUNTING.store(false, Ordering::Relaxed);
    ALLOCATIONS.load(Ordering::Relaxed)
}

#[test]
fn warmed_fault_driven_steps_hold_the_allocation_bounds() {
    assert_eq!(allocations_after_warmup(false), TRIVIAL_STEP_ALLOCATIONS);
    assert_eq!(
        allocations_after_warmup(true),
        STEPS * WRITE_STEP_ALLOCATIONS
    );
}
