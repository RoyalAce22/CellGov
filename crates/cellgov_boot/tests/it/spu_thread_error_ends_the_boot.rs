//! An SPU thread-group error ends a boot as a fault, where the run would
//! otherwise block and read as an exit.

use std::rc::Rc;

use cellgov_boot::manifest::CheckpointTrigger;
use cellgov_boot::step_loop::bench_step_loop;
use cellgov_boot::{BootSink, ChildInitPlans, NullSink};
use cellgov_compare::BootOutcome;
use cellgov_core::Runtime;
use cellgov_exec::{FakeIsaUnit, UnitStatus};
use cellgov_lv2::{GroupState, PendingResponse};
use cellgov_mem::GuestMemory;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

#[test]
fn an_unserved_spu_stop_code_ends_the_bench_loop_as_a_fault() {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(100), 100);
    // `stop 0x3`.
    let spu = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        spu.state_mut().ls[..4].copy_from_slice(&3u32.to_be_bytes());
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
            cause_ptr: 0x100,
            status_ptr: 0x104,
            cause: 0,
            status: 0,
        },
    );

    let sink: Rc<dyn BootSink> = Rc::new(NullSink);
    let mut steps = 0;
    let outcome = bench_step_loop(
        &mut rt,
        CheckpointTrigger::ProcessExit,
        &mut steps,
        &ChildInitPlans::default(),
        &(),
        &sink,
    )
    .expect("the loop runs");
    assert_eq!(outcome, BootOutcome::Fault);
    assert_eq!(
        steps, 1,
        "the loop ends on the step whose commit recorded the error"
    );
}
