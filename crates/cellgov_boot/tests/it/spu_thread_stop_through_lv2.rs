//! A thread-group SPU's stop reaches LV2 through the runtime's commit:
//! `sys_spu_thread_exit` wakes the group's join, and an unserved stop
//! code is a thread-group error the host can take.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::{FakeIsaUnit, UnitStatus};
use cellgov_lv2::{GroupState, PendingResponse, SpuThreadError};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::SPU_WR_OUT_MBOX;
use cellgov_ps3_abi::lv2::spu::group_join_cause;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const CAUSE: u32 = 0x100;
const STATUS: u32 = 0x104;

/// A running one-thread group whose SPU runs `program`, and a PPU
/// stand-in parked on the group's join.
fn group_running(program: &[u32]) -> (Runtime, u32, UnitId, UnitId) {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(100), 100);
    let spu = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        for (i, word) in program.iter().enumerate() {
            spu.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        spu
    });
    let joiner = rt.register_unit_with(|id| FakeIsaUnit::new(id, vec![]));
    let groups = rt.lv2_host_mut().thread_groups_mut();
    let gid = groups.create(1).expect("a group id");
    groups.get_mut(gid).expect("the group").state = GroupState::Running;
    groups
        .record_spu(spu, gid, 0)
        .expect("the SPU joins the group");
    rt.set_unit_status_override(joiner, UnitStatus::Blocked);
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
    (rt, gid, spu, joiner)
}

fn step(rt: &mut Runtime) {
    let step = rt.step().expect("the SPU runs");
    rt.commit_step(&step.result, &step.effects)
        .expect("the step commits");
}

fn word_at(rt: &Runtime, addr: u32) -> u32 {
    let mem = rt.memory().as_bytes();
    let a = addr as usize;
    u32::from_be_bytes([mem[a], mem[a + 1], mem[a + 2], mem[a + 3]])
}

#[test]
fn spu_thread_exit_wakes_the_join_with_all_threads_exit() {
    // `il r3, 7`, `wrch SPU_WrOutMbox, r3`, `stop 0x102`: the open
    // toolchain's `spu_thread_exit(7)`.
    let (mut rt, gid, spu, joiner) = group_running(&[
        (0x081 << 23) | (7 << 7) | 3,
        (0x10D << 21) | (u32::from(SPU_WR_OUT_MBOX) << 7) | 3,
        0x0000_0102,
    ]);
    step(&mut rt);
    assert_eq!(
        rt.registry().effective_status(joiner),
        Some(UnitStatus::Runnable)
    );
    assert_eq!(
        (word_at(&rt, CAUSE), word_at(&rt, STATUS)),
        (group_join_cause::ALL_THREADS_EXIT, 0)
    );
    let group = rt.lv2_host().thread_groups().get(gid).expect("the group");
    assert_eq!(group.thread_exit_status.get(&spu.raw()), Some(&7));
    assert!(rt.take_spu_thread_failure().is_none());
}

#[test]
fn an_unserved_stop_code_faults_the_thread_and_leaves_the_join_parked() {
    let (mut rt, _, spu, joiner) = group_running(&[0x0000_0003]);
    step(&mut rt);
    let failure = rt.take_spu_thread_failure().expect("a thread-group error");
    assert_eq!(
        (failure.unit, failure.error),
        (spu, SpuThreadError::UnservedStopCode(3))
    );
    assert_eq!(
        rt.registry().effective_status(spu),
        Some(UnitStatus::Faulted)
    );
    assert_eq!(
        rt.registry().effective_status(joiner),
        Some(UnitStatus::Blocked)
    );
}
