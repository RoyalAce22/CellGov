//! Decision-log capture from a live runtime: branching appears only with multiple runnable units.

use super::*;
use crate::util::StopReason;
use cellgov_core::Runtime;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_mem::{GuestMemory, PageSize, Region, RegionAccess};
use cellgov_time::Budget;

#[test]
fn two_units_produces_branching_point() {
    let mem = GuestMemory::new(64);
    let mut rt = Runtime::new(mem, Budget::new(100), 100);

    rt.registry_mut()
        .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
    rt.registry_mut()
        .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));

    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::Stalled);
    assert!(
        log.len() >= 2,
        "expected at least 2 steps, got {}",
        log.len()
    );
    assert!(
        log.points()[0].is_branching(),
        "first step should be a branching point (2 runnable)"
    );
    assert!(
        log.branching_count() >= 1,
        "expected at least 1 branching point, got {}",
        log.branching_count()
    );
}

#[test]
fn single_unit_no_branching() {
    let mem = GuestMemory::new(64);
    let mut rt = Runtime::new(mem, Budget::new(100), 100);

    rt.registry_mut()
        .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));

    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::Stalled);
    assert_eq!(log.len(), 1);
    assert_eq!(log.branching_count(), 0);
}

#[test]
fn a_runtime_step_cap_stops_the_observer_with_a_named_reason() {
    let mem = GuestMemory::new(64);
    // Two 3-op units need 6 steps; the cap refuses the 5th.
    let mut rt = Runtime::new(mem, Budget::new(100), 4);
    for imm in [0xAAu32, 0xBB] {
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(imm),
                    FakeOp::SharedStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
    }
    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::StepError);
    assert!(stop.is_truncated());
    assert_eq!(log.len(), 4, "only the committed steps are logged");
}

#[test]
fn a_refused_commit_stops_the_observer_and_names_itself() {
    // A DMA whose destination lands in a reserved region is refused at
    // commit, so the step's effects never reach guest state.
    let mem = GuestMemory::from_regions(vec![
        Region::new(0, 0x10000, "rw", PageSize::Page64K),
        Region::with_access(
            0x10000,
            0x10000,
            "reserved",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap();
    let mut rt = Runtime::new(mem, Budget::new(100), 100);
    rt.registry_mut().register_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::DmaPut {
                    src: 0x40,
                    dst: 0x10000,
                    len: 16,
                },
                FakeOp::End,
            ],
        )
    });

    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::CommitError);
    assert!(stop.is_truncated());
    assert!(
        log.points().is_empty(),
        "a step whose effects were refused must not enter the decision log",
    );
    assert!(
        rt.registry().runnable_ids().next().is_none(),
        "the refused issuer faults, so a leftover-runnable-units test          would read this truncated run as a completed one",
    );
}
