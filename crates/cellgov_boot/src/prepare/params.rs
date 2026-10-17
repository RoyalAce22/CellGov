//! The process parameters a boot reads out of the title ELF -- step
//! budget, primary-thread priority, stack, TLS -- and the decode rules
//! the kernel applies to each declaration.

use cellgov_core::{default_budget_for_mode, RuntimeMode};
use cellgov_ps3_abi::hw::address_space::{
    PS3_PRIMARY_STACK_BASE, PS3_PRIMARY_STACK_ENTRY_RESERVE, PS3_PRIMARY_STACK_SIZE,
    PS3_PRIMARY_STACK_START,
};
use cellgov_ps3_abi::lv2::process::SYS_PROCESS_PARAM_PRIO_LIMIT;
use cellgov_time::Budget;

use super::types::ExecutionOptions;
use crate::BootError;

/// Why the boot could not resolve its process parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ParamsError {
    /// The step cap is below one budget, so no step could run.
    #[error("max_steps={max_steps} below budget={budget}; raise --max-steps or lower --budget")]
    MaxStepsBelowBudget {
        /// The requested retired-instruction cap.
        max_steps: usize,
        /// Retired instructions one `step()` grants.
        budget: u64,
    },
    /// The title declares a primary stack larger than the reservation.
    #[error(
        "primary_stacksize=0x{declared:x} exceeds reserved stack region 0x{reserved:x}; \
         raise PS3_PRIMARY_STACK_SIZE"
    )]
    StackExceedsReservation {
        /// The decoded declaration, in bytes.
        declared: u32,
        /// The reservation it did not fit.
        reserved: usize,
    },
}

/// Default primary-thread priority when the title's `sys_proc_param`
/// block is absent or declares a priority the kernel will not adopt.
///
/// 1001 is the kernel's own starting priority for the primary thread.
const DEFAULT_PRIMARY_PRIO: u32 = 1001;

/// What the boot resolved out of the title's `sys_proc_param` segment
/// and the caller's budget flags.
pub(super) struct BootParams {
    pub tls_info: Option<cellgov_ppu::loader::TlsInfo>,
    /// `None` when the ELF carries no `sys_process_param_t`.
    pub proc_param: Option<cellgov_ppu::loader::SysProcessParam>,
    pub malloc_pagesize: u32,
    pub mode: RuntimeMode,
    pub step_budget: Budget,
    /// `rt.step()` call cap, already divided down from the requested
    /// instruction cap by [`step_call_cap`].
    pub adjusted_max_steps: usize,
    pub primary_prio: u32,
    pub primary_stack_size: u32,
    pub primary_stack_base: u64,
}

/// Read the title's `sys_proc_param` segment and the caller's budget
/// flags into the parameters every later stage reads.
///
/// # Errors
///
/// The step cap is below one budget, or the declared primary stack
/// exceeds the reservation.
pub(super) fn resolve_boot_params(
    execution: &ExecutionOptions<'_>,
    sink: &dyn crate::BootSink,
    elf_data: &[u8],
) -> Result<BootParams, BootError> {
    let tls_info = cellgov_ppu::loader::find_tls_segment(elf_data);
    let proc_param = cellgov_ppu::loader::find_sys_process_param(elf_data);
    let malloc_pagesize = proc_param.map(|p| p.malloc_pagesize).unwrap_or(0x100000);

    let mode = if execution.capture_state_trace {
        RuntimeMode::DeterminismCheck
    } else {
        RuntimeMode::FaultDriven
    };
    let step_budget = {
        let b = execution
            .budget_override
            .unwrap_or_else(|| default_budget_for_mode(mode));
        if b.is_exhausted() {
            // A zero budget stalls the runtime without retiring work
            // (`cellgov_core::Runtime::new` "Zero values"), so the boot
            // would never advance.
            sink.warn("boot: budget 0 retires no work; raised to 1");
            Budget::new(1)
        } else {
            b
        }
    };
    let step_budget_usize = (step_budget.raw() as usize).max(1);
    if execution.runtime_max_steps < step_budget_usize {
        return Err(ParamsError::MaxStepsBelowBudget {
            max_steps: execution.runtime_max_steps,
            budget: step_budget.raw(),
        }
        .into());
    }
    let (adjusted_max_steps, effective_max_steps) =
        step_call_cap(execution.runtime_max_steps, step_budget_usize);
    if effective_max_steps != execution.runtime_max_steps {
        sink.warn(&format!(
            "boot: max_steps={} is not a multiple of budget={step_budget}; \
             the effective cap is {effective_max_steps} retired instructions",
            execution.runtime_max_steps,
        ));
    }

    let primary_prio: u32 = resolve_primary_prio(proc_param.map(|p| p.primary_prio), sink);
    // An adopted declaration is at most 1 MiB, and the reservation holds
    // 1 MiB above the stack start, so the guard below cannot fire.
    let primary_stack_size =
        resolve_primary_stacksize(proc_param.map(|p| p.primary_stacksize), sink);
    let reserved =
        (PS3_PRIMARY_STACK_BASE + PS3_PRIMARY_STACK_SIZE as u64 - PS3_PRIMARY_STACK_START) as usize;
    if (primary_stack_size as usize) > reserved {
        return Err(ParamsError::StackExceedsReservation {
            declared: primary_stack_size,
            reserved,
        }
        .into());
    }
    let primary_stack_base = PS3_PRIMARY_STACK_START;

    Ok(BootParams {
        tls_info,
        proc_param,
        malloc_pagesize,
        mode,
        step_budget,
        adjusted_max_steps,
        primary_prio,
        primary_stack_size,
        primary_stack_base,
    })
}

/// Resolve the primary thread's priority from a `sys_proc_param`
/// declaration.
///
/// An out-of-range declaration is not a load failure: the kernel keeps
/// its own default and boots. A PPU thread priority runs 0 (highest)
/// to 3071. This adopts a declaration only inside that range and at
/// or above the process class's floor, which is 0 for a debug/root
/// process. `cellgov_lv2::PpuThreadAttrs` carries an unsigned
/// priority, so a negative declaration reverts to the default rather
/// than booting at a wrapped value.
fn resolve_primary_prio(declared: Option<i32>, sink: &dyn crate::BootSink) -> u32 {
    let Some(p) = declared else {
        return DEFAULT_PRIMARY_PRIO;
    };
    if (0..SYS_PROCESS_PARAM_PRIO_LIMIT).contains(&p) {
        return p as u32;
    }
    sink.warn(&format!(
        "boot: sys_proc_param primary_prio={p} is outside 0..{SYS_PROCESS_PARAM_PRIO_LIMIT}; \
         the kernel does not adopt it -- using the default {DEFAULT_PRIMARY_PRIO}"
    ));
    DEFAULT_PRIMARY_PRIO
}

/// Decode a `sys_proc_param.primary_stacksize` declaration to bytes.
///
/// A byte count from
/// [`PS3_PRIMARY_STACK_SIZE_MIN`](cellgov_ps3_abi::hw::address_space::PS3_PRIMARY_STACK_SIZE_MIN)
/// through
/// [`PS3_PRIMARY_STACK_SIZE_MAX`](cellgov_ps3_abi::hw::address_space::PS3_PRIMARY_STACK_SIZE_MAX)
/// rounds up to a page. `None` for any other count: the kernel does
/// not adopt it.
fn decode_primary_stacksize(declared: u32) -> Option<u32> {
    use cellgov_ps3_abi::hw::address_space::{
        PS3_PRIMARY_STACK_SIZE_MAX, PS3_PRIMARY_STACK_SIZE_MIN, PS3_STACK_SIZE_GRANULARITY,
    };
    // Both bounds are page multiples, so the round-up cannot push an
    // adopted count past the maximum.
    (PS3_PRIMARY_STACK_SIZE_MIN..=PS3_PRIMARY_STACK_SIZE_MAX)
        .contains(&declared)
        .then(|| declared.next_multiple_of(PS3_STACK_SIZE_GRANULARITY))
}

/// The primary stack size for a title's `primary_stacksize`
/// declaration, or for none.
///
/// A declaration the kernel does not adopt leaves the default, as a
/// priority outside the range does in [`resolve_primary_prio`].
fn resolve_primary_stacksize(declared: Option<u32>, sink: &dyn crate::BootSink) -> u32 {
    use cellgov_ps3_abi::hw::address_space::{
        PS3_PRIMARY_STACK_SIZE_DEFAULT, PS3_PRIMARY_STACK_SIZE_MAX, PS3_PRIMARY_STACK_SIZE_MIN,
    };
    let Some(declared) = declared else {
        return PS3_PRIMARY_STACK_SIZE_DEFAULT;
    };
    decode_primary_stacksize(declared).unwrap_or_else(|| {
        sink.warn(&format!(
            "boot: sys_proc_param primary_stacksize=0x{declared:x} is outside \
             0x{PS3_PRIMARY_STACK_SIZE_MIN:x}..=0x{PS3_PRIMARY_STACK_SIZE_MAX:x}; the kernel \
             does not adopt it -- using the default 0x{PS3_PRIMARY_STACK_SIZE_DEFAULT:x}"
        ));
        PS3_PRIMARY_STACK_SIZE_DEFAULT
    })
}

/// Primary thread r1 at entry for a stack of `size` bytes, with no
/// guest args: [`PS3_PRIMARY_STACK_ENTRY_RESERVE`] below the top of the
/// stack, as on the reference console.
///
/// [CBE-Handbook p:396 s:14.3.1.2] at entry R1 addresses the word
/// holding the initial frame's NULL back-chain pointer.
pub(super) const fn primary_entry_sp(size: u32) -> u64 {
    PS3_PRIMARY_STACK_START + size as u64 - PS3_PRIMARY_STACK_ENTRY_RESERVE
}

/// Convert a retired-instruction cap into the `rt.step()` call cap
/// [`Runtime::new`](cellgov_core::Runtime::new) takes, paired with the
/// instruction cap that actually results.
///
/// `Runtime::max_steps` counts `step()` calls, each granting up to
/// `budget` retired instructions, so a request that is not a multiple
/// of the budget rounds down and the remainder is unreachable.
///
/// # Panics
///
/// `budget` must be non-zero; the caller floors it at 1.
fn step_call_cap(max_instructions: usize, budget: usize) -> (usize, usize) {
    debug_assert!(budget > 0, "caller floors the budget at 1 before dividing");
    let calls = max_instructions / budget;
    (calls, calls * budget)
}

#[cfg(test)]
#[path = "tests/params_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/params_execution_tests.rs"]
mod execution_tests;

#[cfg(test)]
#[path = "tests/primary_stack_console_tests.rs"]
mod primary_stack_console_tests;

#[cfg(test)]
#[path = "tests/primary_stack_placement_console_tests.rs"]
mod primary_stack_placement_console_tests;
