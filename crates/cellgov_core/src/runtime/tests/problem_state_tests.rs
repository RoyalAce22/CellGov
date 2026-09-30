//! A stop request leaves a unit the commit pipeline refused as it is.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, FakeIsaUnit, ProblemStateError,
    UnitStatus,
};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

use super::*;

/// A unit with problem state whose stop request is observable.
#[derive(Clone)]
struct StoppableUnit {
    inner: FakeIsaUnit,
    stopped: bool,
}

impl ExecutionUnit for StoppableUnit {
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

    fn spu_status(&self) -> Option<u32> {
        Some(u32::from(!self.stopped))
    }

    fn request_stop(&mut self, _waiting: bool) -> Result<(), ProblemStateError> {
        self.stopped = true;
        Ok(())
    }
}

#[test]
fn a_stop_request_keeps_the_refusal_the_commit_pipeline_recorded() {
    let mut rt = Runtime::new(GuestMemory::new(4096), Budget::new(1), 1);
    let unit = rt.register_unit_with(|id| StoppableUnit {
        inner: FakeIsaUnit::new(id, vec![]),
        stopped: false,
    });
    rt.registry.set_status_override(unit, UnitStatus::Faulted);
    assert_eq!(rt.request_unit_stop(unit), Ok(()));
    assert_eq!(
        rt.registry.get(unit).and_then(|u| u.spu_status()),
        Some(1),
        "the unit recorded no stop"
    );
    assert_eq!(
        rt.unit_spu_status(unit),
        Some(0),
        "a refused unit reports stopped"
    );
    assert_eq!(
        rt.registry.effective_status(unit),
        Some(UnitStatus::Faulted)
    );

    rt.registry.clear_status_override(unit);
    rt.request_unit_stop(unit).expect("problem state");
    assert_eq!(rt.unit_spu_status(unit), Some(0));
}
