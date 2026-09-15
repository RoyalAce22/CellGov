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

use crate::prx::{
    run_module_start, ModuleStartEnv, ModuleStartError, ModuleStartOutcome, PrxLoadInfo,
};
use crate::BootSink;

/// Why a spawned child's init pass did not finish.
#[derive(Debug, thiserror::Error)]
pub enum ChildInitError {
    /// The runtime parked a child under a token no spawn staged.
    #[error(
        "child module_start: pid 0x{pid:08x} parked under init token {token} that no \
         spawn loader staged; the loader and the runtime disagree"
    )]
    NoPlanForToken {
        /// The child process the runtime parked.
        pid: u32,
        /// The init token the runtime parked it under.
        token: u64,
    },
    /// One of the child's `module_start`s left its unit mid-execution.
    #[error("child pid 0x{pid:08x}: {source}")]
    ModuleStart {
        /// The child process whose module did not return.
        pid: u32,
        /// The refusal the module_start raised.
        #[source]
        source: Box<ModuleStartError>,
    },
}

/// What a spawned child's init pass runs, staged by the spawn loader
/// while it only has the child's memory.
pub(crate) struct ChildInitPlan {
    /// The child's firmware set in topological order, plus any
    /// trampoline pseudo-module.
    pub prx_modules: Vec<PrxLoadInfo>,
    /// Kernel-context OPD installed in the child's space.
    pub kctx_opd: u64,
    /// r1 for each module_start, inside the child's region below its
    /// primary stack.
    pub stack_pointer: u64,
    /// The boot's `disable_module_start_hle_stubs` override, which the child's pass also applies.
    pub run_hle_stubbed: bool,
}

/// Plans staged by the spawn loader, keyed by the token the loader
/// hands the runtime; shared between the loader closure the runtime
/// owns and the step loop that owns the runtime.
#[derive(Clone, Default)]
pub struct ChildInitPlans(Rc<RefCell<Vec<Option<ChildInitPlan>>>>);

impl ChildInitPlans {
    /// Record `plan`; the returned token names it exactly once.
    pub(crate) fn stage(&self, plan: ChildInitPlan) -> u64 {
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
///
/// # Errors
///
/// The runtime parked a child under an unstaged token, or one of the
/// child's `module_start`s left its unit mid-execution.
pub(crate) fn run_pending_child_inits(
    rt: &mut Runtime,
    plans: &ChildInitPlans,
    sink: &Rc<dyn BootSink>,
) -> Result<(), ChildInitError> {
    for pending in rt.take_pending_child_inits() {
        let Some(plan) = plans.take(pending.init_token) else {
            return Err(ChildInitError::NoPlanForToken {
                pid: pending.pid,
                token: pending.init_token,
            });
        };
        let modules_total = plan
            .prx_modules
            .iter()
            .filter(|p| p.module_start.is_some())
            .count();
        sink.note(&format!(
            "child module_start: pid=0x{:08x} space={} modules={} ({} with module_start)",
            pending.pid,
            pending.space.raw(),
            plan.prx_modules.len(),
            modules_total,
        ));

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
            rt.set_unit_status_override(id, UnitStatus::Blocked);
        }

        let env = ModuleStartEnv {
            space: pending.space,
            thread_owner: pending.primary_unit,
            pid: Some(pending.pid),
            kctx_opd: plan.kctx_opd,
            stack_pointer: plan.stack_pointer,
            break_pc: None,
            dump_mem_fault_ranges: Vec::new(),
            run_hle_stubbed: plan.run_hle_stubbed,
            sink: Rc::clone(sink),
        };
        let mut completed: usize = 0;
        let mut faulted: Vec<String> = Vec::new();
        let mut fatal: Option<ModuleStartError> = None;
        for info in &plan.prx_modules {
            match run_module_start(rt, info, &env) {
                Ok(ModuleStartOutcome::Completed { .. }) | Ok(ModuleStartOutcome::HleStubbed) => {
                    completed += 1;
                }
                Ok(ModuleStartOutcome::Skipped) => {}
                Err(ModuleStartError::Faulted { module, .. }) => faulted.push(module),
                Err(e) => {
                    fatal = Some(e);
                    break;
                }
            }
        }

        for (id, prior) in held {
            match prior {
                Some(status) => rt.set_unit_status_override(id, status),
                None => rt.clear_unit_status_override(id),
            }
        }
        if let Some(source) = fatal {
            return Err(ChildInitError::ModuleStart {
                pid: pending.pid,
                source: Box::new(source),
            });
        }
        rt.release_child_init(pending.primary_unit);

        if !faulted.is_empty() {
            sink.warn(&format!(
                "BENCH_CHILD_MODULE_START_FAULTS: pid=0x{:08x} count={} modules={}",
                pending.pid,
                faulted.len(),
                faulted.join(","),
            ));
        }
        sink.note(&format!(
            "child module_start: pid=0x{:08x} completed={completed} faulted={} of {modules_total}",
            pending.pid,
            faulted.len(),
        ));
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
    Ok(())
}

#[cfg(test)]
#[path = "tests/child_init_tests.rs"]
mod tests;
