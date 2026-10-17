//! The RSX context goes with the process that allocated it.

use super::*;
use crate::host::process::ProcessEntry;
use crate::host::rsx::test_helpers::context_allocate_request;
use crate::host::test_support::FakeRuntime;
use cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID;

const CHILD: u32 = BOOT_PROCESS_PID + 0x100;
const PARENT_UNIT: UnitId = UnitId::new(0);
const CHILD_UNIT: UnitId = UnitId::new(1);

fn allocate(host: &mut Lv2Host, unit: UnitId) -> Lv2Dispatch {
    host.dispatch(
        context_allocate_request(0x1000, 0x1008, 0x1010, 0x1018, 1),
        unit,
        &FakeRuntime::new(0x1_0000),
    )
}

#[test]
fn a_childs_exit_tears_down_the_context_it_allocated() {
    let mut host = Lv2Host::new();
    assert!(host.state.processes.insert_child(
        CHILD,
        ProcessEntry {
            ppid: BOOT_PROCESS_PID,
            authority_id: 0,
            control_flags1: 0,
            exit_status: None,
            module_registered: false,
        },
    ));
    host.bind_unit_process(CHILD_UNIT, CHILD);
    assert!(matches!(
        allocate(&mut host, CHILD_UNIT),
        Lv2Dispatch::Immediate { code: 0, .. }
    ));
    let queue = host.sys_rsx_context().event_queue_id;

    host.mark_process_exited(CHILD, 0);

    let ctx = host.sys_rsx_context();
    assert!(!ctx.allocated, "the context went with its process");
    assert_eq!(ctx.control_slot(0), None, "its window closed");
    assert!(
        host.state.event_queues.lookup(queue).is_none(),
        "and its queue was dropped"
    );
    assert!(
        matches!(
            allocate(&mut host, PARENT_UNIT),
            Lv2Dispatch::Immediate { code: 0, .. }
        ),
        "another process can allocate a context again"
    );
}

#[test]
fn a_childs_exit_leaves_a_context_the_parent_allocated() {
    let mut host = Lv2Host::new();
    assert!(host.state.processes.insert_child(
        CHILD,
        ProcessEntry {
            ppid: BOOT_PROCESS_PID,
            authority_id: 0,
            control_flags1: 0,
            exit_status: None,
            module_registered: false,
        },
    ));
    host.bind_unit_process(CHILD_UNIT, CHILD);
    assert!(matches!(
        allocate(&mut host, PARENT_UNIT),
        Lv2Dispatch::Immediate { code: 0, .. }
    ));
    // The child's own queue is the one its exit drops.
    let child_queue = host.dispatch(
        crate::request::classify(
            cellgov_ps3_abi::lv2::syscall::EVENT_QUEUE_CREATE,
            &[0x2000, 0x2100, 0, 8, 0, 0, 0, 0],
        ),
        CHILD_UNIT,
        &FakeRuntime::new(0x1_0000),
    );
    assert!(matches!(
        child_queue,
        Lv2Dispatch::Immediate { code: 0, .. }
    ));

    host.mark_process_exited(CHILD, 0);

    assert!(host.sys_rsx_context().allocated);
}
