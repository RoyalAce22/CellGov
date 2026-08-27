//! Module_start pass for spawned children.
//!
//! The spawn loader loads a child's firmware closure into the child's
//! space and stages a [`ChildInitPlan`]; the runtime parks the child's
//! primary thread behind a `PendingChildInit`; the step loops call
//! [`run_pending_child_inits`] between steps to run the plan's
//! module_starts inside the child and release the primary. The same
//! `run_module_start` drives both processes, so a child's init is
//! witnessed the way the boot's is.

use std::cell::RefCell;
use std::rc::Rc;

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;

use crate::cli::exit::die;
use crate::game::prx::PrxLoadInfo;
use crate::game::prx::{run_module_start, ModuleStartEnv, ModuleStartError, ModuleStartOutcome};

/// What a spawned child's init pass runs, staged by the spawn loader
/// while it only has the child's memory.
pub(in crate::game) struct ChildInitPlan {
    /// The child's firmware set in topological order, plus any
    /// trampoline pseudo-module.
    pub(in crate::game) prx_modules: Vec<PrxLoadInfo>,
    /// Kernel-context OPD installed in the child's space.
    pub(in crate::game) kctx_opd: u64,
    /// r1 for each module_start, inside the child's region below its
    /// primary stack.
    pub(in crate::game) stack_pointer: u64,
}

/// Plans staged by the spawn loader, keyed by the token the loader
/// hands the runtime; shared between the loader closure the runtime
/// owns and the step loop that owns the runtime.
#[derive(Clone, Default)]
pub(in crate::game) struct ChildInitPlans(Rc<RefCell<Vec<Option<ChildInitPlan>>>>);

impl ChildInitPlans {
    /// Record `plan`; the returned token names it exactly once.
    pub(in crate::game) fn stage(&self, plan: ChildInitPlan) -> u64 {
        let mut plans = self.0.borrow_mut();
        plans.push(Some(plan));
        (plans.len() - 1) as u64
    }

    fn take(&self, token: u64) -> Option<ChildInitPlan> {
        let mut plans = self.0.borrow_mut();
        let slot = usize::try_from(token).ok()?;
        plans.get_mut(slot).and_then(Option::take)
    }
}

/// Run every parked child's init pass and release its primary.
///
/// Every other runnable unit is held `Blocked` for the pass so the
/// module_starts schedule as the boot's do: one transient unit at a
/// time.
pub(in crate::game) fn run_pending_child_inits(rt: &mut Runtime, plans: &ChildInitPlans) {
    for pending in rt.take_pending_child_inits() {
        let Some(plan) = plans.take(pending.init_token) else {
            die(&format!(
                "child module_start: pid 0x{:08x} parked under init token {} that no \
                 spawn loader staged; the loader and the runtime disagree",
                pending.pid, pending.init_token,
            ));
        };
        let modules_total = plan
            .prx_modules
            .iter()
            .filter(|p| p.module_start.is_some())
            .count();
        println!(
            "child module_start: pid=0x{:08x} space={} modules={} ({} with module_start)",
            pending.pid,
            pending.space.raw(),
            plan.prx_modules.len(),
            modules_total,
        );

        let held: Vec<(UnitId, Option<UnitStatus>)> = rt
            .registry()
            .ids()
            .filter(|&id| {
                id != pending.primary_unit
                    && rt.registry().effective_status(id) == Some(UnitStatus::Runnable)
            })
            .map(|id| (id, rt.registry().status_override(id)))
            .collect();
        for &(id, _) in &held {
            rt.registry_mut()
                .set_status_override(id, UnitStatus::Blocked);
        }

        let env = ModuleStartEnv {
            space: pending.space,
            thread_owner: pending.primary_unit,
            pid: Some(pending.pid),
            kctx_opd: plan.kctx_opd,
            stack_pointer: plan.stack_pointer,
            break_pc: None,
            dump_mem_fault_ranges: Vec::new(),
        };
        let mut completed: usize = 0;
        let mut faulted: Vec<String> = Vec::new();
        for info in &plan.prx_modules {
            match run_module_start(rt, info, &env) {
                Ok(ModuleStartOutcome::Completed { .. }) | Ok(ModuleStartOutcome::HleStubbed) => {
                    completed += 1;
                }
                Ok(ModuleStartOutcome::Skipped) => {}
                Err(ModuleStartError::Faulted { module, .. }) => faulted.push(module),
                Err(e) => die(&format!("child pid 0x{:08x}: {e}", pending.pid)),
            }
        }

        for (id, prior) in held {
            match prior {
                Some(status) => rt.registry_mut().set_status_override(id, status),
                None => rt.registry_mut().clear_status_override(id),
            }
        }
        rt.release_child_init(pending.primary_unit);

        if !faulted.is_empty() {
            eprintln!(
                "BENCH_CHILD_MODULE_START_FAULTS: pid=0x{:08x} count={} modules={}",
                pending.pid,
                faulted.len(),
                faulted.join(","),
            );
        }
        println!(
            "child module_start: pid=0x{:08x} completed={completed} faulted={} of {modules_total}",
            pending.pid,
            faulted.len(),
        );
        debug_assert_eq!(
            completed + faulted.len(),
            modules_total,
            "child module_start: completed {completed} + faulted {} of {modules_total}",
            faulted.len(),
        );

        // The child's modules are not entered in the process-shared
        // PRX registry (keyed by stem, already holding the boot's set),
        // so a by-path load from inside the child resolves against
        // the boot's modules.
        let real_modules = plan
            .prx_modules
            .iter()
            .filter(|p| !p.stem.is_empty())
            .count();
        if real_modules > 0 {
            rt.lv2_host_mut().log_invariant_break(
                "process.child_prx_registry_shared",
                format_args!(
                    "pid 0x{:08x} loaded {real_modules} firmware module(s) into space {} \
                     that the process-shared PRX registry and export map do not describe",
                    pending.pid,
                    pending.space.raw(),
                ),
            );
        }
    }
}

#[cfg(test)]
#[path = "tests/child_init_tests.rs"]
mod tests;
