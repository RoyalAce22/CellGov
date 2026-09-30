//! The commit holds a fenced or barrier command behind the queued
//! commands its ordering names, whatever the latency model says.

use cellgov_dma::{DmaDirection, DmaLatencyModel, DmaQueue, DmaRequest, MfcOrdering};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::MfcTagId;
use cellgov_time::{Budget, GuestTicks, InstructionCost};

use crate::runtime::state::Runtime;

/// A get takes 100 ticks and a put 10, so a later put would complete
/// first unless something orders it.
struct SlowGets;

impl DmaLatencyModel for SlowGets {
    fn completion_time(&self, req: &DmaRequest, now: GuestTicks, _: &DmaQueue) -> GuestTicks {
        let ticks = match req.direction() {
            DmaDirection::Get => 100,
            DmaDirection::Put => 10,
        };
        now.saturating_add(GuestTicks::new(ticks))
    }
}

/// Emits `requests` in one step, then finishes.
#[derive(Clone)]
struct Issuer {
    id: UnitId,
    requests: Vec<(DmaDirection, u8, MfcOrdering)>,
    done: bool,
}

impl ExecutionUnit for Issuer {
    type Snapshot = bool;
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
        _budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.done = true;
        for (i, &(direction, tag, ordering)) in self.requests.iter().enumerate() {
            let main = ByteRange::new(GuestAddr::new(0x40 + 0x10 * i as u64), 4).expect("fits");
            let local = ByteRange::new(GuestAddr::new(0x100), 4).expect("fits");
            let (src, dst, payload) = match direction {
                DmaDirection::Put => (local, main, Some(vec![0x5A; 4])),
                DmaDirection::Get => (main, local, None),
            };
            let request = DmaRequest::new(direction, src, dst, self.id)
                .expect("equal lengths")
                .with_tag_id(MfcTagId::new(tag).expect("in range"))
                .with_ordering(ordering);
            effects.push(Effect::DmaEnqueue { request, payload });
        }
        ExecutionStepResult {
            yield_reason: YieldReason::DmaSubmitted,
            consumed_cost: InstructionCost::new(1),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }
    fn snapshot(&self) -> bool {
        self.done
    }
}

/// The completion times of the queued commands, earliest first.
fn completion_times(requests: Vec<(DmaDirection, u8, MfcOrdering)>) -> Vec<u64> {
    let mut rt = Runtime::new(GuestMemory::new(0x200), Budget::new(4), 100);
    rt.dma_latency = Box::new(SlowGets);
    rt.registry_mut().register_with(|id| Issuer {
        id,
        requests,
        done: false,
    });
    let step = rt.step().expect("the issuer runs");
    rt.commit_step(&step.result, &step.effects)
        .expect("the enqueues commit");
    let mut times: Vec<u64> = rt
        .dma_queue()
        .pending()
        .map(|(c, _)| c.completion_time().raw())
        .collect();
    times.sort_unstable();
    times
}

/// [CBEA p:69 s:7.9] a tag-specific fence orders the command after every preceding command in its tag group.
#[test]
fn a_fenced_put_completes_after_the_get_it_follows() {
    use DmaDirection::{Get, Put};
    let fenced = completion_times(vec![
        (Get, 1, MfcOrdering::None),
        (Put, 1, MfcOrdering::Fence),
    ]);
    let plain = completion_times(vec![
        (Get, 1, MfcOrdering::None),
        (Put, 1, MfcOrdering::None),
    ]);
    assert_eq!(plain[0] + 90, plain[1], "unordered, the put finishes first");
    assert_eq!(fenced[0], fenced[1], "fenced, the put waits for the get");
}

#[test]
fn a_fence_in_another_tag_group_is_not_held() {
    use DmaDirection::{Get, Put};
    let times = completion_times(vec![
        (Get, 1, MfcOrdering::None),
        (Put, 2, MfcOrdering::Fence),
    ]);
    assert_eq!(times[0] + 90, times[1]);
}
