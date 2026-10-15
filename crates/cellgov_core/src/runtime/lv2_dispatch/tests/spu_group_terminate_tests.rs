//! `sys_spu_thread_group_terminate` through the runtime: the group's
//! SPUs finish, a parked join wakes with the terminate cause and value,
//! and the caller reads 0.

use cellgov_event::UnitId;
use cellgov_exec::{FakeIsaUnit, UnitStatus};
use cellgov_lv2::{GroupState, Lv2Request, PendingResponse};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv2::spu::group_join_cause;
use cellgov_time::Budget;

use crate::runtime::Runtime;

const CAUSE: u32 = 0x100;
const STATUS: u32 = 0x104;

/// A running two-thread group with its second SPU parked in a wait, a
/// PPU parked on the group's join, and the PPU that terminates the group.
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
            cause_ptr: CAUSE,
            status_ptr: STATUS,
            cause: 0,
            status: 0,
        },
    );
    rt.registry
        .set_status_override(spus[1], UnitStatus::Blocked);
    let _ = rt
        .syscall_responses_mut()
        .insert(spus[1], PendingResponse::ReturnCode { code: 0 });
    (rt, spus, joiner, caller, gid)
}

fn word_at(rt: &Runtime, addr: u32) -> u32 {
    let mem = rt.memory().as_bytes();
    let a = addr as usize;
    u32::from_be_bytes([mem[a], mem[a + 1], mem[a + 2], mem[a + 3]])
}

#[test]
fn terminate_finishes_every_spu_and_wakes_the_join_with_terminated_and_the_value() {
    let (mut rt, spus, joiner, caller, gid) = running_group();
    rt.dispatch_lv2_request(
        Lv2Request::SpuThreadGroupTerminate {
            group_id: gid,
            value: 0x7E57,
        },
        caller,
    );
    for spu in spus {
        assert_eq!(
            rt.registry.effective_status(spu),
            Some(UnitStatus::Finished),
            "{spu:?}"
        );
    }
    assert!(
        rt.syscall_responses_mut().try_take(spus[1]).is_none(),
        "the parked SPU's wait is gone"
    );
    assert_eq!(
        rt.registry.effective_status(joiner),
        Some(UnitStatus::Runnable)
    );
    assert_eq!(
        (word_at(&rt, CAUSE), word_at(&rt, STATUS)),
        (group_join_cause::TERMINATED, 0x7E57)
    );
    assert_eq!(rt.registry.drain_syscall_return(caller), Some(0));
}

#[test]
fn a_refused_terminate_finishes_nothing() {
    let (mut rt, spus, joiner, caller, gid) = running_group();
    rt.dispatch_lv2_request(
        Lv2Request::SpuThreadGroupTerminate {
            group_id: gid + 1,
            value: 0,
        },
        caller,
    );
    assert_eq!(
        rt.registry.drain_syscall_return(caller),
        Some(cellgov_ps3_abi::lv2::errno::CELL_ESRCH.into())
    );
    assert_eq!(rt.registry.status_override(spus[0]), None);
    assert_eq!(
        rt.registry.effective_status(joiner),
        Some(UnitStatus::Blocked)
    );
}
