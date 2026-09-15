//! The commit pipeline validates a DMA's ends in the memory the
//! transfer reads and writes.
//!
//! A transfer stays in space 0 end to end, whatever space its issuer
//! runs in, so the enqueue check resolves its ends there too. Against
//! the issuer's own space that check has two failures:
//!
//! - it refuses a transfer whose ends resolve in space 0;
//! - it accepts one whose completion then meets an unmapped range.

use cellgov_dma::{DmaDirection, DmaRequest};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize};
use cellgov_time::{Budget, InstructionCost};

use crate::commit::CommitError;
use crate::runtime::spaces::AddressSpaceId;
use crate::runtime::state::Runtime;

const S1: AddressSpaceId = AddressSpaceId::new(1);

/// Space 0's own addresses. The child space maps neither.
const SPACE0_SRC: u64 = 0x20;
const SPACE0_DST: u64 = 0x40;

/// Where the child space's only region sits. Space 0 does not map it.
const CHILD_ONLY: u64 = 0x2000;

const LEN: u64 = 4;

/// The bytes the transfer moves, so the destination says whether it
/// ran.
const PAYLOAD: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];

fn range(addr: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), LEN).expect("a 4-byte range")
}

/// Enqueues one payload-less put, parks on it, and finishes once the
/// completion wakes it.
///
/// Payload-less puts the source end under test. An inline payload
/// never reads guest memory, so only a payload-less transfer has a
/// source range to resolve. The destination check is the same for
/// both. The park gives the warp a unit to wake, so the completion
/// fires inside the run, not at a final drain.
#[derive(Clone)]
struct DmaEmitter {
    id: UnitId,
    src: u64,
    dst: u64,
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
            let request =
                DmaRequest::new(DmaDirection::Put, range(self.src), range(self.dst), self.id)
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

/// A runtime whose child space maps only [`CHILD_ONLY`], and whose
/// space 0 maps only the low region holding [`SPACE0_SRC`] and
/// [`SPACE0_DST`].
///
/// The two region sets are disjoint, so each address resolves in
/// exactly one space. That is what makes a check against the wrong
/// space fail.
fn child_space_emitter(src: u64, dst: u64) -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(0x100), Budget::new(4), 100);
    rt.create_address_space(S1).expect("a fresh space id");
    rt.space_memory_mut(S1)
        .expect("the space was just created")
        .install_region(CHILD_ONLY, 0x100, "child", PageSize::Page64K)
        .expect("a region the child space does not already map");
    rt.registry_mut().register_with(|id| DmaEmitter {
        id,
        src,
        dst,
        steps: 0,
    });
    rt.assign_unit_space(UnitId::new(0), S1)
        .expect("the unit and the space both exist");
    rt
}

/// Without this premise the cases below could pass against a runtime
/// whose spaces map the same bytes. That is the one shape that would
/// hide the defect.
#[test]
fn the_two_spaces_map_disjoint_addresses() {
    let rt = child_space_emitter(SPACE0_SRC, SPACE0_DST);
    let child = rt.space_memory(S1).expect("the child space exists");
    assert!(
        rt.memory().read(range(SPACE0_SRC)).is_some()
            && rt.memory().read(range(SPACE0_DST)).is_some(),
        "space 0 has to map the transfer's ends",
    );
    assert!(
        child.read(range(SPACE0_SRC)).is_none(),
        "the child space must not map the space-0 source, or the old \
         reading would resolve too",
    );
    assert!(
        child.read(range(CHILD_ONLY)).is_some() && rt.memory().read(range(CHILD_ONLY)).is_none(),
        "and the child-only address must resolve only in the child",
    );
}

#[test]
fn a_child_space_transfer_over_space_zero_ends_is_accepted_and_completes() {
    let mut rt = child_space_emitter(SPACE0_SRC, SPACE0_DST);
    rt.place_bytes(AddressSpaceId::BOOT, range(SPACE0_SRC), &PAYLOAD)
        .expect("space 0 maps the source");

    let step = rt.step().expect("the emitter runs");
    rt.commit_step(&step.result, &step.effects)
        .expect("the enqueue resolves in the memory the transfer uses");
    // The enqueue queues a completion and writes nothing, so the
    // destination is still zero here. The payload read below is
    // therefore evidence of a completion, not of the placement or of a
    // write by the issuer.
    assert_eq!(
        rt.memory().read(range(SPACE0_DST)).map(<[u8]>::to_vec),
        Some(vec![0u8; LEN as usize]),
        "the destination carried the payload before any completion fired",
    );

    // Nothing is runnable now, so the runtime warps to the completion.
    for _ in 0..4 {
        let Ok(step) = rt.step() else { break };
        rt.commit_step(&step.result, &step.effects)
            .expect("no later step of this workload refuses its commit");
    }

    assert!(
        rt.dma_queue().is_empty(),
        "the completion never came due, so the destination says nothing \
         about where the transfer would have landed",
    );
    assert_eq!(
        rt.memory().read(range(SPACE0_DST)).map(<[u8]>::to_vec),
        Some(PAYLOAD.to_vec()),
        "the completion writes the destination in space 0",
    );
}

/// Accepting this is what leaves the completion's read to meet an
/// unmapped range.
#[test]
fn a_child_space_transfer_over_child_only_ends_is_refused_at_enqueue() {
    let mut rt = child_space_emitter(CHILD_ONLY, CHILD_ONLY + 0x80);
    let step = rt.step().expect("the emitter runs");
    let err = rt
        .commit_step(&step.result, &step.effects)
        .expect_err("space 0 maps neither end");
    assert!(
        matches!(err, CommitError::DmaSourceOutOfRange { .. }),
        "the source is the first end the check resolves: {err:?}",
    );
    assert!(
        rt.dma_queue().is_empty(),
        "a refused enqueue queues no completion, so nothing is left to \
         meet the transfer's read with an unmapped range",
    );
    assert_eq!(
        rt.registry().effective_status(UnitId::new(0)),
        Some(UnitStatus::Faulted),
        "the issuer yielded DmaWait, and a refused batch leaves it \
         faulted rather than parked on a completion it threw away",
    );
}

/// The case above stops at the source, so it never reaches the
/// destination check.
///
/// The destination is also the end an acceptance cannot survive:
/// `apply_dma_transfer` host-writes it into space 0 and treats a
/// refusal there as impossible.
#[test]
fn a_child_space_transfer_over_a_child_only_destination_is_refused_at_enqueue() {
    let mut rt = child_space_emitter(SPACE0_SRC, CHILD_ONLY + 0x80);
    let step = rt.step().expect("the emitter runs");
    let err = rt
        .commit_step(&step.result, &step.effects)
        .expect_err("space 0 maps the source but not the destination");
    assert!(
        matches!(err, CommitError::DmaDestinationOutOfRange { .. }),
        "the source resolves, so the destination is what refuses: {err:?}",
    );
    assert!(
        rt.dma_queue().is_empty(),
        "a refused enqueue queues no completion, so no completion is \
         left to host-write an unmapped destination",
    );
}
