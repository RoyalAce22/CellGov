//! A DMA's ends translate in the memory the transfer reads and writes.
//!
//! A transfer stays in space 0 end to end, whatever space its issuer
//! runs in, so the queue resolves its ends there when it reaches the
//! transfer.
//! Against the issuer's own space that check has two failures:
//!
//! - it raises a transfer whose ends resolve in space 0;
//! - it completes one whose ends do not.

use cellgov_dma::{DmaDirection, DmaRequest};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize};
use cellgov_time::{Budget, InstructionCost};

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
        .expect("the enqueue passes its checks");
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

/// Runs every step until the runtime stops.
fn run_out(rt: &mut Runtime) {
    for _ in 0..8 {
        let Ok(step) = rt.step() else { break };
        rt.commit_step(&step.result, &step.effects)
            .expect("no step of this workload refuses its commit");
    }
}

/// The source resolves only in the child, so space 0 has nothing to
/// read.
#[test]
fn a_child_space_transfer_over_a_child_only_source_raises_in_space_zero() {
    let mut rt = child_space_emitter(CHILD_ONLY, SPACE0_DST);
    run_out(&mut rt);
    let exception = rt.take_mfc_exception().expect("the queue raised it");
    assert_eq!(
        exception.command.error,
        cellgov_dma::MfcCommandError::DataStorage { ea: CHILD_ONLY }
    );
    assert_eq!(
        rt.memory().read(range(SPACE0_DST)).map(<[u8]>::to_vec),
        Some(vec![0u8; LEN as usize]),
        "a raised transfer moves nothing",
    );
}

/// The destination is the end a completion writes, so it is the one an
/// accepted transfer cannot land in.
#[test]
fn a_child_space_transfer_over_a_child_only_destination_raises_in_space_zero() {
    let mut rt = child_space_emitter(SPACE0_SRC, CHILD_ONLY + 0x80);
    rt.place_bytes(AddressSpaceId::BOOT, range(SPACE0_SRC), &PAYLOAD)
        .expect("space 0 maps the source");
    run_out(&mut rt);
    let exception = rt.take_mfc_exception().expect("the queue raised it");
    assert_eq!(
        exception.command.error,
        cellgov_dma::MfcCommandError::DataStorage {
            ea: CHILD_ONLY + 0x80
        }
    );
}
