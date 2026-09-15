//! A batch that did not apply moves no RSX cursor.
//!
//! The mirror reads committed memory rather than the effect payload, so
//! a discarded batch that covers a control-register slot would project
//! whatever already sat there. The cursor would then advance on a step
//! whose writes never landed.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestMemory, PageSize};
use cellgov_time::{Budget, GuestTicks, InstructionCost};

use crate::rsx::control_register::PUT_ADDR;
use crate::runtime::spaces::AddressSpaceId;
use crate::runtime::state::Runtime;

/// What committed memory already holds at the put slot, and what the
/// cursor must not pick up from it.
const STALE_PUT: u32 = 0x1234;

fn put_range() -> ByteRange {
    ByteRange::contiguous_u32(PUT_ADDR, 4)
}

/// Writes the put slot once and yields `reason`.
#[derive(Clone)]
struct SlotWriter {
    id: UnitId,
    reason: YieldReason,
    done: bool,
}

impl ExecutionUnit for SlotWriter {
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
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.done = true;
        effects.push(Effect::shared_write(
            put_range(),
            WritePayload::new(0xABCDu32.to_be_bytes().to_vec()),
            self.id,
            GuestTicks::ZERO,
        ));
        ExecutionStepResult {
            yield_reason: self.reason,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) {}
}

/// A runtime with the RSX region mapped, the mirror on, and the put
/// slot already holding [`STALE_PUT`] while the cursor still reads
/// zero.
///
/// The two disagreeing is what makes a projection visible: a mirror
/// that ran would move the cursor to the slot's value.
fn runtime_with_stale_put(reason: YieldReason) -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(0x100), Budget::new(4), 100);
    rt.memory_mut()
        .install_region(u64::from(PUT_ADDR), 0x100, "rsx", PageSize::Page64K)
        .expect("the RSX region does not overlap the low region");
    rt.place_bytes(AddressSpaceId::BOOT, put_range(), &STALE_PUT.to_be_bytes())
        .expect("the slot is mapped and writable");
    rt.set_rsx_mirror_writes(true);
    rt.registry_mut().register_with(|id| SlotWriter {
        id,
        reason,
        done: false,
    });
    rt
}

/// The premise: the slot and the cursor disagree before the step.
#[test]
fn the_put_slot_and_the_cursor_start_apart() {
    let rt = runtime_with_stale_put(YieldReason::Finished);
    assert_eq!(rt.rsx_cursor().put(), 0, "the cursor starts at zero");
    assert_eq!(
        rt.memory().read(put_range()).map(<[u8]>::to_vec),
        Some(STALE_PUT.to_be_bytes().to_vec()),
        "and the slot already holds something else",
    );
}

/// A faulting batch is discarded, so its write never lands and the
/// cursor keeps its own value.
#[test]
fn a_discarded_batch_leaves_the_cursor_alone() {
    let mut rt = runtime_with_stale_put(YieldReason::Fault);
    let step = rt.step().expect("the writer runs");
    rt.commit_step(&step.result, &step.effects)
        .expect("a faulting batch is discarded rather than refused");
    assert_eq!(
        rt.memory().read(put_range()).map(<[u8]>::to_vec),
        Some(STALE_PUT.to_be_bytes().to_vec()),
        "the discarded write never reached the slot",
    );
    assert_eq!(
        rt.rsx_cursor().put(),
        0,
        "the mirror reads committed memory, so running it over a batch \
         that applied nothing would project the stale slot into the cursor",
    );
}

/// The same write in a batch that applies does move the cursor.
///
/// Without this the case above would pass against a mirror that never
/// runs at all.
#[test]
fn an_applied_batch_moves_the_cursor() {
    let mut rt = runtime_with_stale_put(YieldReason::Finished);
    let step = rt.step().expect("the writer runs");
    rt.commit_step(&step.result, &step.effects)
        .expect("the slot is mapped and writable");
    assert_eq!(
        rt.rsx_cursor().put(),
        0xABCD,
        "an applied write to the put slot projects into the cursor",
    );
}
