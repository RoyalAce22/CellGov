//! `sys_spu_thread_group_suspend` and `sys_spu_thread_group_resume`
//! through the runtime: a suspended group's units leave the runnable set
//! whatever their status, a wake that lands while they are held shows
//! once they are released, and a terminate ends a suspended group.

use cellgov_event::UnitId;
use cellgov_exec::{FakeIsaUnit, UnitStatus};
use cellgov_lv2::thread_group::MAX_SLOTS_PER_GROUP;
use cellgov_lv2::{GroupState, Lv2Request, PendingResponse};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

use crate::runtime::Runtime;

/// A running two-thread group, its second SPU parked in a wait, a PPU
/// parked on the group's join, and the PPU that drives the group.
fn running_group() -> (Runtime, [UnitId; 2], UnitId, UnitId, u32) {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(1), 10);
    let spus = [0, 1].map(|_| rt.register_unit_with(|id| FakeIsaUnit::new(id, vec![])));
    let joiner = rt.register_unit_with(|id| FakeIsaUnit::new(id, vec![]));
    let caller = rt.register_unit_with(|id| FakeIsaUnit::new(id, vec![]));
    let groups = rt.lv2_host_mut().thread_groups_mut();
    let gid = groups.create(2).expect("a group id");
    groups.get_mut(gid).expect("the group").state = GroupState::Running;
    for (slot, &unit) in spus.iter().enumerate() {
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
            cause_ptr: 0x100,
            status_ptr: 0x104,
            cause: 0,
            status: 0,
        },
    );
    rt.registry
        .set_status_override(spus[1], UnitStatus::Blocked);
    (rt, spus, joiner, caller, gid)
}

fn suspend(rt: &mut Runtime, caller: UnitId, group_id: u32) -> Option<u64> {
    rt.dispatch_lv2_request(Lv2Request::SpuThreadGroupSuspend { group_id }, caller);
    rt.registry.drain_syscall_return(caller)
}

fn resume(rt: &mut Runtime, caller: UnitId, group_id: u32) -> Option<u64> {
    rt.dispatch_lv2_request(Lv2Request::SpuThreadGroupResume { group_id }, caller);
    rt.registry.drain_syscall_return(caller)
}

#[test]
fn suspend_holds_every_spu_out_of_the_runnable_set_and_resume_restores_their_status() {
    let (mut rt, spus, _, caller, gid) = running_group();
    let before = rt.registry.status_hash();
    assert_eq!(suspend(&mut rt, caller, gid), Some(0));
    for spu in spus {
        assert_eq!(
            rt.registry.effective_status(spu),
            Some(UnitStatus::Suspended),
            "{spu:?}"
        );
    }
    assert!(rt.registry.runnable_ids().all(|id| !spus.contains(&id)));
    assert_ne!(rt.registry.status_hash(), before);

    assert_eq!(resume(&mut rt, caller, gid), Some(0));
    assert_eq!(
        rt.registry.effective_status(spus[0]),
        Some(UnitStatus::Runnable)
    );
    assert_eq!(
        rt.registry.effective_status(spus[1]),
        Some(UnitStatus::Blocked)
    );
    assert_eq!(rt.registry.status_hash(), before);
}

#[test]
fn a_wake_while_suspended_shows_after_the_resume() {
    let (mut rt, spus, _, caller, gid) = running_group();
    assert_eq!(suspend(&mut rt, caller, gid), Some(0));
    rt.registry
        .set_status_override(spus[1], UnitStatus::Runnable);
    assert_eq!(
        rt.registry.effective_status(spus[1]),
        Some(UnitStatus::Suspended)
    );
    assert_eq!(resume(&mut rt, caller, gid), Some(0));
    assert_eq!(
        rt.registry.effective_status(spus[1]),
        Some(UnitStatus::Runnable)
    );
}

#[test]
fn a_mailbox_message_written_while_suspended_ends_the_park_after_the_resume() {
    let (mut rt, spus, _, caller, gid) = running_group();
    let mailbox = cellgov_sync::MailboxId::new(spus[1].raw());
    assert!(rt.mailbox_registry_mut().register_at(mailbox, 4));
    assert_eq!(suspend(&mut rt, caller, gid), Some(0));
    rt.dispatch_lv2_request(
        Lv2Request::SpuThreadWriteMb {
            thread_id: gid * MAX_SLOTS_PER_GROUP + 1,
            value: 0x5A,
        },
        caller,
    );
    assert_eq!(rt.registry.drain_syscall_return(caller), Some(0));
    assert_eq!(
        rt.registry.effective_status(spus[1]),
        Some(UnitStatus::Suspended),
        "the held SPU stays held"
    );
    assert_eq!(resume(&mut rt, caller, gid), Some(0));
    assert_eq!(
        rt.registry.effective_status(spus[1]),
        Some(UnitStatus::Runnable),
        "the message delivered during the hold ends the park"
    );
}

#[test]
fn a_terminate_ends_a_suspended_group_and_wakes_its_join() {
    let (mut rt, spus, joiner, caller, gid) = running_group();
    assert_eq!(suspend(&mut rt, caller, gid), Some(0));
    rt.dispatch_lv2_request(
        Lv2Request::SpuThreadGroupTerminate {
            group_id: gid,
            value: 2,
        },
        caller,
    );
    assert_eq!(rt.registry.drain_syscall_return(caller), Some(0));
    for spu in spus {
        assert_eq!(
            rt.registry.effective_status(spu),
            Some(UnitStatus::Finished),
            "{spu:?}"
        );
    }
    assert_eq!(
        rt.registry.effective_status(joiner),
        Some(UnitStatus::Runnable)
    );
    let group = rt.lv2_host().thread_groups().get(gid).expect("the group");
    assert!(!group.suspended);
    assert_eq!(group.state, GroupState::Finished);
}

#[test]
fn a_refused_suspend_holds_nothing() {
    let (mut rt, spus, _, caller, gid) = running_group();
    assert_eq!(
        suspend(&mut rt, caller, gid + 1),
        Some(cellgov_ps3_abi::lv2::errno::CELL_ESRCH.into())
    );
    assert_eq!(
        rt.registry.effective_status(spus[0]),
        Some(UnitStatus::Runnable)
    );
}
