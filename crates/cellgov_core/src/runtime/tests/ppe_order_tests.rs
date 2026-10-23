//! Which PPE sibling's issue interval reaches a thread: only one created
//! before it, whatever the two threads' LV2 priorities.

use crate::runtime::state::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::{FakeIsaUnit, FakeOp, PpeIssuePressure};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

/// The pressure every unit reports: an issue every ten cycles.
const PRESSURE: PpeIssuePressure = PpeIssuePressure {
    issue_interval: 10 * 256,
    ..PpeIssuePressure::NONE
};

fn attrs(priority: u32) -> cellgov_lv2::PpuThreadAttrs {
    cellgov_lv2::PpuThreadAttrs {
        entry: 0,
        arg: 0,
        stack_base: 0,
        stack_size: 0,
        priority,
        tls_base: 0,
    }
}

/// A runtime of two PPE units that report [`PRESSURE`], each with an LV2
/// thread record at its entry of `priorities` when that is given, both
/// runnable.
fn runtime(priorities: [Option<u32>; 2]) -> (Runtime, [UnitId; 2]) {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(1), 10_000);
    let program = vec![FakeOp::SetPpePressure(PRESSURE), FakeOp::LoadImm(1)];
    let mut unit = || {
        let p = program.clone();
        rt.register_unit_with(|id| FakeIsaUnit::new(id, p).as_ppe_thread())
    };
    let ids = [unit(), unit()];
    if let Some(first) = priorities[0] {
        rt.lv2_host_mut()
            .seed_primary_ppu_thread(ids[0], attrs(first));
    }
    if let Some(second) = priorities[1] {
        rt.lv2_host_mut()
            .ppu_threads_mut()
            .create(ids[1], attrs(second))
            .expect("a thread id");
    }
    for _ in 0..4 {
        let Ok(step) = rt.step() else { break };
        rt.commit_unchecked(&step.result, &step.effects)
            .expect("the step commits");
    }
    for id in ids {
        let unit = rt.registry().get(id).expect("a registered unit");
        assert_eq!(
            unit.ppe_issue_pressure(),
            PRESSURE,
            "{id:?} set its pressure"
        );
    }
    (rt, ids)
}

/// The issue interval each of `ids` receives from the other, with both
/// in the runnable set.
fn received(rt: &Runtime, ids: [UnitId; 2]) -> [u32; 2] {
    ids.map(|id| {
        let (siblings, pressure) = rt.ppe_siblings(id, &ids);
        assert_eq!(siblings, 1);
        pressure.issue_interval
    })
}

#[test]
fn the_later_thread_receives_the_earlier_one_s_issue_interval_at_every_priority() {
    // The `ppu_issue_priority` capture's three phases: the later thread
    // below, at and above the primary's priority.
    for later in [2000, 1001, 500] {
        let (rt, ids) = runtime([Some(1001), Some(later)]);
        assert_eq!(
            received(&rt, ids),
            [0, PRESSURE.issue_interval],
            "later thread at {later}"
        );
    }
}

#[test]
fn units_with_no_thread_record_take_the_same_order() {
    let (rt, ids) = runtime([None, None]);
    assert_eq!(received(&rt, ids), [0, PRESSURE.issue_interval]);
}
