//! A DMA landing invalidates predecoded code where its bytes land.
//!
//! The destination and every shared-view alias the fanout replicated
//! into hold new bytes, so a unit caching decoded instructions there
//! has to re-decode, the way it does after a committed store.

use std::cell::RefCell;
use std::rc::Rc;

use cellgov_dma::{DmaDirection, DmaRequest};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, InstructionCost};

use crate::runtime::spaces::AddressSpaceId;
use crate::runtime::state::Runtime;

const S1: AddressSpaceId = AddressSpaceId::new(1);

const KEY: u64 = 7;
const SEG_SIZE: u64 = 0x40;
const VIEW0_BASE: u64 = 0x2000;
const VIEW1_BASE: u64 = 0x3000;

/// The transfer's source, outside the segment, and its destination
/// inside view 0.
const SRC: u64 = 0x20;
const DEST: u64 = VIEW0_BASE + 0x10;
const LEN: u64 = 8;

fn range(addr: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), LEN).expect("an 8-byte range")
}

/// Enqueues one put to [`DEST`], parks, and finishes once the
/// completion wakes it.
#[derive(Clone)]
struct DmaEmitter {
    id: UnitId,
    steps: u64,
}

impl ExecutionUnit for DmaEmitter {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps >= 2 {
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
        self.steps += 1;
        let yield_reason = if self.steps == 1 {
            let request = DmaRequest::new(DmaDirection::Put, range(SRC), range(DEST), self.id)
                .expect("equal-length ends");
            effects.push(Effect::DmaEnqueue {
                request,
                payload: None,
            });
            YieldReason::DmaWait
        } else {
            YieldReason::Finished
        };
        ExecutionStepResult {
            yield_reason,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) {}
}

/// The `(addr, len)` of every `invalidate_code` call a recorder saw.
type Calls = Rc<RefCell<Vec<(u64, u64)>>>;

/// Emits nothing; records every `invalidate_code` call it receives.
#[derive(Clone)]
struct InvalidationRecorder {
    id: UnitId,
    calls: Calls,
    done: bool,
}

impl ExecutionUnit for InvalidationRecorder {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.done {
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
        self.done = true;
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn invalidate_code(&mut self, addr: u64, len: u64) {
        self.calls.borrow_mut().push((addr, len));
    }

    fn caches_code(&self) -> bool {
        true
    }

    fn snapshot(&self) {}
}

/// An emitter and a recorder over a runtime whose space 0 maps
/// [`DEST`]; `shared` adds a second view of the segment in a child
/// space.
fn build(shared: bool) -> (Runtime, Calls) {
    // A shared segment installs its own region at `VIEW0_BASE`, so the
    // base region stops short of it; without one, the base region is
    // what maps the destination.
    let mem_bytes = if shared { 0x100 } else { 0x3000 };
    let mut rt = Runtime::new(GuestMemory::new(mem_bytes), Budget::new(4), 100);
    if shared {
        rt.create_address_space(S1).expect("a fresh space id");
        rt.register_shared_mapping(
            KEY,
            SEG_SIZE,
            &[(AddressSpaceId::BOOT, VIEW0_BASE), (S1, VIEW1_BASE)],
        )
        .expect("two views of one fresh key");
    }
    rt.registry_mut()
        .register_with(|id| DmaEmitter { id, steps: 0 });
    let calls = Rc::new(RefCell::new(Vec::new()));
    let recorder_calls = Rc::clone(&calls);
    rt.registry_mut()
        .register_with(move |id| InvalidationRecorder {
            id,
            calls: recorder_calls,
            done: false,
        });
    rt.place_bytes(AddressSpaceId::BOOT, range(SRC), &[0xA5; LEN as usize])
        .expect("space 0 maps the source");
    (rt, calls)
}

/// Runs until the completion fires.
fn run_to_completion(rt: &mut Runtime) {
    for _ in 0..6 {
        let Ok(step) = rt.step() else { break };
        rt.commit_step(&step.result, &step.effects)
            .expect("no step of this workload refuses its commit");
    }
    assert!(
        rt.dma_queue().is_empty(),
        "the completion never came due, so nothing landed",
    );
}

#[test]
fn a_landing_invalidates_code_at_its_destination() {
    let (mut rt, calls) = build(false);
    run_to_completion(&mut rt);

    assert_eq!(
        *calls.borrow(),
        vec![(DEST, LEN)],
        "the destination range, once, and no alias where no view is shared",
    );
}

#[test]
fn a_landing_in_a_shared_view_invalidates_every_alias() {
    let (mut rt, calls) = build(true);
    run_to_completion(&mut rt);

    let alias = VIEW1_BASE + (DEST - VIEW0_BASE);
    assert_eq!(
        *calls.borrow(),
        vec![(DEST, LEN), (alias, LEN)],
        "the destination and its sibling-view alias, at the alias's own base",
    );
}

/// The premise: nothing else in this workload invalidates. A commit
/// with no `SharedWriteIntent` reaches the recorder only through the
/// landing.
#[test]
fn nothing_invalidates_before_the_completion_lands() {
    let (mut rt, calls) = build(false);
    let step = rt.step().expect("the emitter runs first");
    rt.commit_step(&step.result, &step.effects)
        .expect("the enqueue commits");
    assert!(
        step.effects
            .iter()
            .any(|e| matches!(e, Effect::DmaEnqueue { .. })),
        "the first step is the enqueue",
    );
    assert!(
        calls.borrow().is_empty(),
        "an enqueue moves no bytes, so it invalidates nothing: {:?}",
        calls.borrow(),
    );
}
