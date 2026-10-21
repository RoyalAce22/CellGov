//! At an SPU thread's exit, the runtime also lands the transfers that a
//! held list queues when earlier landings release it.

use cellgov_dma::{DmaCompletion, DmaDirection, DmaRequest};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, MfcOrderingView,
    UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::MfcTagId;
use cellgov_time::{Budget, GuestTicks, InstructionCost};

use crate::runtime::state::Runtime;

const TAG: u8 = 1;
const HELD_DESTINATION: u64 = 0x80;

fn put(issuer: UnitId, destination: u64) -> DmaRequest {
    DmaRequest::new(
        DmaDirection::Put,
        ByteRange::new(GuestAddr::new(0x100), 4).expect("fits"),
        ByteRange::new(GuestAddr::new(destination), 4).expect("fits"),
        issuer,
    )
    .expect("equal lengths")
    .with_tag_id(MfcTagId::new(TAG).expect("in range"))
}

/// Holds one fenced list until its tag group has no queued command, then
/// starts it as one stall-and-notify element, which holds no slot.
#[derive(Clone)]
struct HoldsList {
    id: UnitId,
    started: bool,
}

impl ExecutionUnit for HoldsList {
    type Snapshot = bool;
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        UnitStatus::Finished
    }
    fn run_until_yield(
        &mut self,
        _budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(1),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }
    fn start_held_lists(&mut self, view: MfcOrderingView) -> Vec<Effect> {
        let bit = MfcTagId::new(TAG).expect("in range").status_bit();
        if self.started || view.outstanding_tags & bit != 0 {
            return Vec::new();
        }
        self.started = true;
        let request = put(self.id, HELD_DESTINATION)
            .with_stall_notify()
            .without_slot();
        vec![Effect::DmaEnqueue {
            request,
            payload: Some(vec![0x5A; 4]),
        }]
    }
    fn snapshot(&self) -> bool {
        self.started
    }
}

#[test]
fn the_exit_drain_lands_a_released_list_element_that_holds_no_slot() {
    let mut rt = Runtime::new(GuestMemory::new(0x200), Budget::new(4), 100);
    let id = rt
        .registry_mut()
        .register_with(|id| HoldsList { id, started: false });
    rt.dma_queue.enqueue(
        DmaCompletion::new(put(id, 0x40), GuestTicks::new(50)),
        Some(vec![0xA5; 4]),
    );
    let landed = rt.land_issuer_dma(id);
    assert_eq!(
        landed.len(),
        2,
        "the queued put and the list element it released both land"
    );
    assert!(
        rt.dma_queue.is_empty(),
        "no element of the released list stays queued past the exit"
    );
    let range = ByteRange::new(GuestAddr::new(HELD_DESTINATION), 4).expect("fits");
    assert_eq!(rt.memory.read(range).expect("in memory"), &[0x5A; 4]);
}
