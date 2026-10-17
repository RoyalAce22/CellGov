//! An SPU thread's exit lands the transfers the thread still has
//! queued before the group's join wakes, and leaves every other unit's
//! transfers to their modeled time.

use cellgov_dma::{DmaCompletion, DmaDirection, DmaRequest};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, FakeIsaUnit, ProblemStateError,
    StopRegisters, UnitStatus,
};
use cellgov_lv2::{GroupState, PendingResponse};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, GuestTicks};

use super::*;

const LATE: u64 = 1_000_000;

/// An SPU thread stand-in that has stopped with `code` and holds the
/// exit status 0 in its outbound mailbox.
#[derive(Clone)]
struct ExitingUnit {
    inner: FakeIsaUnit,
    code: u32,
    out_mbox: Option<u32>,
}

impl ExecutionUnit for ExitingUnit {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.inner.unit_id()
    }

    fn status(&self) -> UnitStatus {
        UnitStatus::Finished
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.inner.run_until_yield(budget, ctx, effects)
    }

    fn snapshot(&self) {}

    fn stop_registers(&self) -> Option<StopRegisters> {
        Some(StopRegisters {
            status: (self.code << 16) | 0b10,
            npc: 0x40,
        })
    }

    fn read_out_mbox(&mut self) -> Result<Option<u32>, ProblemStateError> {
        Ok(self.out_mbox.take())
    }
}

/// A put of four 0xAB bytes from `issuer` to `at` that completes at
/// `LATE`.
fn queue_put(rt: &mut Runtime, issuer: UnitId, at: u64) {
    let request = DmaRequest::new(
        DmaDirection::Put,
        ByteRange::new(GuestAddr::new(0), 4).expect("source"),
        ByteRange::new(GuestAddr::new(at), 4).expect("destination"),
        issuer,
    )
    .expect("a request");
    rt.dma_queue.enqueue(
        DmaCompletion::new(request, GuestTicks::new(LATE)),
        Some(vec![0xAB; 4]),
    );
}

fn byte_at(rt: &Runtime, at: u64) -> u8 {
    rt.memory().as_bytes()[at as usize]
}

/// One group of one exiting thread, a bystander unit with a put of its
/// own queued, and a PPU parked on the join; the exiting thread has a
/// put to 0x200 queued, the bystander one to 0x300.
fn exiting_group(code: u32) -> (Runtime, UnitId, UnitId) {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(1), 10);
    let spu = rt.register_unit_with(|id| ExitingUnit {
        inner: FakeIsaUnit::new(id, vec![]),
        code,
        out_mbox: Some(0),
    });
    let bystander = rt.register_unit_with(|id| FakeIsaUnit::new(id, vec![]));
    let joiner = rt.register_unit_with(|id| FakeIsaUnit::new(id, vec![]));
    let groups = rt.lv2_host_mut().thread_groups_mut();
    let gid = groups.create(1).expect("a group id");
    groups.get_mut(gid).expect("the group").state = GroupState::Running;
    groups.record_spu(spu, gid, 0).expect("the SPU joins");
    rt.registry.set_status_override(joiner, UnitStatus::Blocked);
    let _ = rt.syscall_responses_mut().insert(
        joiner,
        PendingResponse::ThreadGroupJoin {
            group_id: gid,
            code: 0,
            cause_ptr: 0x100,
            status_ptr: 0x104,
            cause: 0,
            status: 0,
        },
    );
    queue_put(&mut rt, spu, 0x200);
    queue_put(&mut rt, bystander, 0x300);
    (rt, spu, joiner)
}

fn exit(code: u32) {
    let (mut rt, spu, joiner) = exiting_group(code);
    let stopped = rt.registry.get(spu).and_then(|u| u.stop_registers());
    let landed = rt.settle_finished_unit(spu, stopped);
    assert_eq!(
        rt.registry.effective_status(joiner),
        Some(UnitStatus::Runnable),
        "the join returns"
    );
    assert_eq!(byte_at(&rt, 0x200), 0xAB, "the thread's put has landed");
    assert_eq!(byte_at(&rt, 0x300), 0, "the bystander's put waits");
    assert_eq!(rt.dma_queue.len(), 1);
    assert_eq!(
        landed.iter().map(|(c, _)| c.issuer()).collect::<Vec<_>>(),
        [spu],
        "the landing is reported for the commit"
    );
}

#[test]
fn a_thread_exit_lands_the_threads_transfers_before_the_join_wakes() {
    exit(0x102);
}

#[test]
fn a_group_exit_lands_the_threads_transfers_before_the_join_wakes() {
    exit(0x101);
}
