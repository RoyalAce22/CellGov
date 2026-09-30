//! LV2's reading of a stopped SPU thread: exits finish it and wake a
//! join with the group's cause and status, a yield resumes it, and
//! anything else is a thread-group error the host can take.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, FakeIsaUnit, ProblemStateError,
    RestartError, StopRegisters, UnitStatus,
};
use cellgov_lv2::{GroupState, PendingResponse, SpuThreadError};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv2::spu::group_join_cause;
use cellgov_time::Budget;

use super::*;

const CAUSE: u32 = 0x100;
const STATUS: u32 = 0x104;

/// An SPU stand-in: the stop word it reports, its outbound mailbox,
/// and whether a restart reached it. Its own status is `Runnable`, so
/// a status the runtime reports for it is the runtime's override.
#[derive(Clone)]
struct StoppedUnit {
    inner: FakeIsaUnit,
    status: u32,
    out_mbox: Option<u32>,
    restarted: bool,
}

impl ExecutionUnit for StoppedUnit {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.inner.unit_id()
    }

    fn status(&self) -> UnitStatus {
        UnitStatus::Runnable
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
            status: self.status,
            npc: 0x40,
        })
    }

    fn restart(&mut self) -> Result<(), RestartError> {
        self.restarted = true;
        Ok(())
    }

    fn read_out_mbox(&mut self) -> Result<Option<u32>, ProblemStateError> {
        Ok(self.out_mbox.take())
    }
}

/// The `SPU_Status` word of a stop-and-signal with `code`.
fn stop_word(code: u32) -> u32 {
    (code << 16) | 0b10
}

/// A running group of `stops.len()` SPU threads, one per stop word and
/// outbound mailbox, and a PPU parked on its join.
fn group_of(stops: &[(u32, Option<u32>)]) -> (Runtime, Vec<UnitId>, UnitId) {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(1), 10);
    let units: Vec<UnitId> = stops
        .iter()
        .map(|&(status, out_mbox)| {
            rt.register_unit_with(|id| StoppedUnit {
                inner: FakeIsaUnit::new(id, vec![]),
                status,
                out_mbox,
                restarted: false,
            })
        })
        .collect();
    let joiner = rt.register_unit_with(|id| FakeIsaUnit::new(id, vec![]));
    let groups = rt.lv2_host_mut().thread_groups_mut();
    let gid = groups.create(stops.len() as u32).expect("a group id");
    groups.get_mut(gid).expect("the group").state = GroupState::Running;
    for (slot, &unit) in units.iter().enumerate() {
        groups
            .record_spu(unit, gid, slot as u32)
            .expect("the SPU joins");
    }
    rt.registry.set_status_override(joiner, UnitStatus::Blocked);
    let _ = rt.syscall_responses_mut().insert(
        joiner,
        PendingResponse::ThreadGroupJoin {
            group_id: gid,
            code: 0,
            cause_ptr: CAUSE,
            status_ptr: STATUS,
            cause: 0xDEAD_BEEF,
            status: 0xCAFE_BABE,
        },
    );
    (rt, units, joiner)
}

fn settle(rt: &mut Runtime, unit: UnitId) {
    let stopped = rt.registry.get(unit).and_then(|u| u.stop_registers());
    rt.settle_finished_unit(unit, stopped);
}

fn word_at(rt: &Runtime, addr: u32) -> u32 {
    let mem = rt.memory().as_bytes();
    let a = addr as usize;
    u32::from_be_bytes([mem[a], mem[a + 1], mem[a + 2], mem[a + 3]])
}

#[test]
fn the_last_thread_exit_wakes_the_join_with_all_threads_exit_and_keeps_each_status() {
    let (mut rt, units, joiner) =
        group_of(&[(stop_word(0x102), Some(7)), (stop_word(0x102), Some(8))]);
    settle(&mut rt, units[0]);
    assert_eq!(
        rt.registry.effective_status(joiner),
        Some(UnitStatus::Blocked)
    );
    settle(&mut rt, units[1]);
    assert_eq!(
        rt.registry.effective_status(joiner),
        Some(UnitStatus::Runnable)
    );
    assert_eq!(
        (word_at(&rt, CAUSE), word_at(&rt, STATUS)),
        (group_join_cause::ALL_THREADS_EXIT, 0)
    );
    let group = rt.lv2_host().thread_groups().get(1).expect("the group");
    assert_eq!(
        group
            .thread_exit_status
            .values()
            .copied()
            .collect::<Vec<_>>(),
        [7, 8]
    );
}

#[test]
fn a_group_exit_ends_every_thread_and_wakes_the_join_with_its_status() {
    let (mut rt, units, joiner) =
        group_of(&[(stop_word(0x101), Some(9)), (stop_word(0x102), None)]);
    settle(&mut rt, units[0]);
    assert_eq!(
        rt.registry.effective_status(joiner),
        Some(UnitStatus::Runnable)
    );
    assert_eq!(
        (word_at(&rt, CAUSE), word_at(&rt, STATUS)),
        (group_join_cause::GROUP_EXIT, 9)
    );
    assert_eq!(
        rt.registry.effective_status(units[1]),
        Some(UnitStatus::Finished)
    );
    assert!(rt.take_spu_thread_failure().is_none());
}

#[test]
fn a_yield_resumes_the_thread_and_finishes_nothing() {
    let (mut rt, units, joiner) = group_of(&[(stop_word(0x100), None)]);
    settle(&mut rt, units[0]);
    let unit = rt.registry.get(units[0]).expect("the SPU");
    let spu = unit
        .as_any()
        .downcast_ref::<StoppedUnit>()
        .expect("a StoppedUnit");
    assert!(spu.restarted);
    assert_eq!(
        rt.registry.effective_status(joiner),
        Some(UnitStatus::Blocked)
    );
}

// [CBEA p:94 s:8.5.2] H (bit 29) reports a halt.
#[test]
fn an_error_stop_faults_the_thread_leaves_the_join_parked_and_is_taken_once() {
    let (mut rt, units, joiner) = group_of(&[(1 << 2, None)]);
    settle(&mut rt, units[0]);
    assert_eq!(
        rt.registry.effective_status(units[0]),
        Some(UnitStatus::Faulted)
    );
    assert_eq!(
        rt.registry.effective_status(joiner),
        Some(UnitStatus::Blocked)
    );
    let failure = rt.take_spu_thread_failure().expect("a failure");
    assert_eq!(
        (failure.unit, failure.error),
        (units[0], SpuThreadError::Halt)
    );
    assert!(
        failure.to_string().contains("halt instruction"),
        "{failure}"
    );
    assert!(rt.take_spu_thread_failure().is_none());
}

#[test]
fn an_exit_with_an_empty_outbound_mailbox_is_an_error() {
    let (mut rt, units, joiner) = group_of(&[(stop_word(0x102), None)]);
    settle(&mut rt, units[0]);
    assert_eq!(
        rt.take_spu_thread_failure().map(|f| f.error),
        Some(SpuThreadError::ExitWithoutStatus(0x102))
    );
    assert_eq!(
        rt.registry.effective_status(joiner),
        Some(UnitStatus::Blocked)
    );
}

#[test]
fn a_stop_from_a_unit_no_group_holds_is_no_lv2_request() {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(1), 10);
    let unit = rt.register_unit_with(|id| StoppedUnit {
        inner: FakeIsaUnit::new(id, vec![]),
        status: 1 << 2,
        out_mbox: None,
        restarted: false,
    });
    settle(&mut rt, unit);
    assert!(rt.take_spu_thread_failure().is_none());
    assert_eq!(rt.registry.status_override(unit), None);
}

// [CBEA p:92 s:8.5.1] a stop request stops the SPU's instruction issue; the page says nothing of the MFC.
#[test]
fn a_transfer_that_completes_while_its_issuer_is_stopped_keeps_its_tag_bit() {
    use cellgov_dma::{DmaCompletion, DmaDirection, DmaRequest};
    use cellgov_mem::{ByteRange, GuestAddr};
    use cellgov_ps3_abi::hw::spu::MfcTagId;
    use cellgov_time::GuestTicks;

    let (mut rt, units, _) = group_of(&[(stop_word(0x100), None)]);
    let issuer = units[0];
    rt.registry
        .set_status_override(issuer, UnitStatus::Finished);
    let request = DmaRequest::new(
        DmaDirection::Put,
        ByteRange::new(GuestAddr::new(0), 4).expect("source"),
        ByteRange::new(GuestAddr::new(0x200), 4).expect("destination"),
        issuer,
    )
    .expect("a request")
    .with_tag_id(MfcTagId::new(3).expect("tag 3"));
    rt.dma_queue
        .enqueue(DmaCompletion::new(request, GuestTicks::ZERO), None);
    rt.fire_dma_completions();
    assert_eq!(
        rt.pending_tag_completions.get(&issuer).copied(),
        Some(1 << 3)
    );
    assert_eq!(
        rt.registry.effective_status(issuer),
        Some(UnitStatus::Finished)
    );
}
