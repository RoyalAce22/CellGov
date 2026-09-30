//! A transfer's effective address translates when the queue reaches the
//! transfer. The enqueue checks no address.
//!
//! [CBEA p:118 s:9.1.6] the address's validity is checked asynchronous to the instruction stream, during the transfer.

use cellgov_dma::{DmaDirection, DmaRequest, MfcCommandError};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize};
use cellgov_ps3_abi::hw::ppu::CELL_EA_LIMIT;
use cellgov_time::{Budget, InstructionCost};

use crate::runtime::state::Runtime;

const LEN: u64 = 4;

/// Where a test maps a region after the unit queues its put.
const LATE: u64 = 0x2000;

fn range(addr: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), LEN).expect("a 4-byte range")
}

/// Puts `len` bytes, carried inline, to each address in turn, one per
/// step, then finishes.
#[derive(Clone)]
struct Putter {
    id: UnitId,
    destinations: Vec<u64>,
    len: u64,
    steps: usize,
}

impl ExecutionUnit for Putter {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps > self.destinations.len() {
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
        let yield_reason = match self.destinations.get(self.steps) {
            Some(&dst) => {
                let span = |addr| ByteRange::new(GuestAddr::new(addr), self.len).expect("fits");
                let request = DmaRequest::new(DmaDirection::Put, span(0), span(dst), self.id)
                    .expect("equal-length ends");
                effects.push(Effect::DmaEnqueue {
                    request,
                    payload: Some(vec![0xA5; self.len as usize]),
                });
                YieldReason::DmaSubmitted
            }
            None => YieldReason::Finished,
        };
        self.steps += 1;
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

fn runtime_putting(destinations: Vec<u64>) -> Runtime {
    runtime_putting_len(destinations, LEN)
}

fn runtime_putting_len(destinations: Vec<u64>, len: u64) -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(0x100), Budget::new(4), 100);
    rt.registry_mut().register_with(|id| Putter {
        id,
        destinations,
        len,
        steps: 0,
    });
    rt
}

/// Runs every step, then drains the queue.
fn run_out(rt: &mut Runtime) {
    while let Ok(step) = rt.step() {
        rt.commit_step(&step.result, &step.effects)
            .expect("no step of this workload refuses its commit");
    }
    rt.drain_pending_dma();
}

/// [CBEA p:120 s:9.1.7] a segment fault raises the MFC data-segment interrupt; a mapping fault raises the MFC data-storage interrupt.
#[test]
fn a_put_past_the_space_is_a_segment_fault_and_one_inside_with_no_region_a_storage_fault() {
    for (dst, want) in [
        (
            CELL_EA_LIMIT - 1,
            MfcCommandError::DataSegment {
                ea: CELL_EA_LIMIT - 1,
            },
        ),
        (0x1000, MfcCommandError::DataStorage { ea: 0x1000 }),
    ] {
        let mut rt = runtime_putting(vec![dst]);
        run_out(&mut rt);
        let exception = rt.take_mfc_exception().expect("the queue raised the put");
        assert_eq!(exception.command.error, want, "put to 0x{dst:x}");
        assert_eq!(exception.command.params.ea(), dst);
        assert_eq!(exception.command.params.size, LEN as u32);
    }
}

#[test]
fn an_address_mapped_before_the_queue_reaches_the_put_translates() {
    let mut rt = runtime_putting(vec![LATE]);
    let step = rt.step().expect("the putter runs");
    rt.commit_step(&step.result, &step.effects)
        .expect("the enqueue is accepted");
    assert_eq!(rt.dma_queue().len(), 1, "the put is still queued");
    rt.memory_mut()
        .install_region(LATE, 0x100, "late", PageSize::Page64K)
        .expect("nothing maps it yet");
    run_out(&mut rt);
    assert_eq!(rt.take_mfc_exception(), None);
    assert_eq!(
        rt.memory().read(range(LATE)).map(<[u8]>::to_vec),
        Some(vec![0xA5; LEN as usize])
    );
}

/// The raised put suspends its issuer's queue.
#[test]
fn a_raised_put_moves_nothing_and_holds_the_issuers_later_puts() {
    let mut rt = runtime_putting(vec![0x1000, 0x40]);
    run_out(&mut rt);
    assert!(rt.take_mfc_exception().is_some());
    assert_eq!(
        rt.memory().read(range(0x40)).map(<[u8]>::to_vec),
        Some(vec![0; LEN as usize]),
        "the later put waits behind the suspended queue"
    );
    assert_eq!(rt.dma_queue().len(), 2, "both commands stay queued");
}

/// A put of no bytes touches no address, whatever address it names.
///
/// [CBEA p:116 s:9.1.4 MFC Transfer Size or List Size Channel] Zero is a valid MFC transfer size.
#[test]
fn a_zero_byte_put_to_an_address_no_region_backs_completes() {
    let mut rt = runtime_putting_len(vec![0x1000, CELL_EA_LIMIT + 1], 0);
    run_out(&mut rt);
    assert_eq!(rt.take_mfc_exception(), None);
    assert!(rt.dma_queue().is_empty());
}
