//! The process parameters a boot reads out of the title ELF -- step
//! budget, primary-thread priority, stack, TLS -- and the decode rules
//! the kernel applies to each declaration.

use cellgov_core::{default_budget_for_mode, RuntimeMode};
use cellgov_ps3_abi::process_address_space::{
    PS3_PRIMARY_STACK_BASE, PS3_PRIMARY_STACK_SIZE, PS3_PRIMARY_STACK_TOP,
};
use cellgov_ps3_abi::sys_process::SYS_PROCESS_PARAM_PRIO_LIMIT;
use cellgov_time::Budget;

use super::types::{u32_or_die, PrepareOptions};
use crate::cli::exit::die;

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

pub(super) fn resolve_boot_params(opts: &PrepareOptions<'_>, elf_data: &[u8]) -> BootParams {
    let tls_info = cellgov_ppu::loader::find_tls_segment(elf_data);
    let proc_param = cellgov_ppu::loader::find_sys_process_param(elf_data);
    let malloc_pagesize = proc_param.map(|p| p.malloc_pagesize).unwrap_or(0x100000);

    let mode = if opts.capture_state_trace {
        RuntimeMode::DeterminismCheck
    } else {
        RuntimeMode::FaultDriven
    };
    let step_budget = {
        let b = opts
            .budget_override
            .unwrap_or_else(|| default_budget_for_mode(mode));
        if b.is_exhausted() {
            // A zero budget stalls the runtime without retiring work
            // (`cellgov_core::Runtime::new` "Zero values"), so the boot
            // would never advance.
            eprintln!("boot: budget 0 retires no work; raised to 1");
            Budget::new(1)
        } else {
            b
        }
    };
    let step_budget_usize = (step_budget.raw() as usize).max(1);
    if opts.runtime_max_steps < step_budget_usize {
        die(&format!(
            "max_steps={} below budget={step_budget}; raise --max-steps or lower --budget",
            opts.runtime_max_steps
        ));
    }
    let (adjusted_max_steps, effective_max_steps) =
        step_call_cap(opts.runtime_max_steps, step_budget_usize);
    if effective_max_steps != opts.runtime_max_steps {
        eprintln!(
            "boot: max_steps={} is not a multiple of budget={step_budget}; \
             the effective cap is {effective_max_steps} retired instructions",
            opts.runtime_max_steps,
        );
    }

    let primary_prio: u32 = resolve_primary_prio(proc_param.map(|p| p.primary_prio));
    // An absent param segment leaves the kernel's own starting value
    // of 1 MiB, which is also the ceiling on a `sys_proc_param` stack
    // declaration. `decode_primary_stacksize` clamps a present
    // declaration to that same ceiling, so the guard below cannot
    // fire while the reservation is 1 MiB.
    let primary_stack_size: u32 = match proc_param {
        Some(p) => {
            let want = decode_primary_stacksize(p.primary_stacksize);
            if (want as usize) > PS3_PRIMARY_STACK_SIZE {
                die(&format!(
                    "primary_stacksize=0x{want:x} exceeds reserved stack region 0x{:x}; \
                     raise PS3_PRIMARY_STACK_SIZE",
                    PS3_PRIMARY_STACK_SIZE
                ));
            }
            want
        }
        None => u32_or_die("PS3_PRIMARY_STACK_SIZE", PS3_PRIMARY_STACK_SIZE as u64),
    };
    let primary_stack_base = primary_stack_base_for(primary_stack_size);

    BootParams {
        tls_info,
        proc_param,
        malloc_pagesize,
        mode,
        step_budget,
        adjusted_max_steps,
        primary_prio,
        primary_stack_size,
        primary_stack_base,
    }
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
fn resolve_primary_prio(declared: Option<i32>) -> u32 {
    let Some(p) = declared else {
        return DEFAULT_PRIMARY_PRIO;
    };
    if (0..SYS_PROCESS_PARAM_PRIO_LIMIT).contains(&p) {
        return p as u32;
    }
    eprintln!(
        "boot: sys_proc_param primary_prio={p} is outside 0..{SYS_PROCESS_PARAM_PRIO_LIMIT}; \
         the kernel does not adopt it -- using the default {DEFAULT_PRIMARY_PRIO}"
    );
    DEFAULT_PRIMARY_PRIO
}

/// Decode a `sys_proc_param.primary_stacksize` declaration to bytes.
///
/// The field carries either a kernel sentinel or a raw byte count.
/// This clamps a raw count between
/// [`PS3_PRIMARY_STACK_SIZE_MIN`](cellgov_ps3_abi::process_address_space::PS3_PRIMARY_STACK_SIZE_MIN)
/// and
/// [`PS3_PRIMARY_STACK_SIZE_MAX`](cellgov_ps3_abi::process_address_space::PS3_PRIMARY_STACK_SIZE_MAX)
/// -- 64 KiB through 1 MiB -- and rounds it up to a page. The field's
/// own declared floor is 4 KiB; the clamp uses the wider 64 KiB floor
/// the kernel gives every process.
fn decode_primary_stacksize(declared: u32) -> u32 {
    use cellgov_ps3_abi::process_address_space::{
        PS3_PRIMARY_STACK_SIZE_MAX, PS3_PRIMARY_STACK_SIZE_MIN, PS3_STACK_SIZE_GRANULARITY,
    };
    match declared {
        0x10 => 32 * 1024,
        0x20 => 64 * 1024,
        0x30 => 96 * 1024,
        0x40 => 128 * 1024,
        0x50 => 256 * 1024,
        0x60 => 512 * 1024,
        0x70 => 1024 * 1024,
        // Both clamp bounds are page multiples, so the round-up
        // cannot push the result past the maximum.
        raw => raw
            .clamp(PS3_PRIMARY_STACK_SIZE_MIN, PS3_PRIMARY_STACK_SIZE_MAX)
            .next_multiple_of(PS3_STACK_SIZE_GRANULARITY),
    }
}

/// Primary thread r1 for a boot with no guest args.
///
/// [`PS3_PRIMARY_STACK_TOP`] already sits
/// [`PS3_ABI_MIN_STACK_FRAME`](cellgov_ps3_abi::process_address_space::PS3_ABI_MIN_STACK_FRAME)
/// below the end of the primary stack region, so the no-args entry
/// takes it unchanged -- subtracting the reserve a second time here
/// would drop r1 a whole frame below the initial frame.
///
/// [CBE-Handbook p:396 s:14.3.1.2] at entry R1 addresses the word
/// holding the initial frame's NULL back-chain pointer.
pub(super) const fn primary_entry_sp() -> u64 {
    PS3_PRIMARY_STACK_TOP
}

/// Base address recorded for the primary thread's stack of `size`
/// bytes.
///
/// [CBE-Handbook p:396 s:14.3.1.3] the loader initializes the stack
/// before entry and hands the program its address in R1, with the
/// argument information block above it.
///
/// CellGov's SP is the fixed [`PS3_PRIMARY_STACK_TOP`], so the
/// recorded base sits `size` below the top of the reservation rather
/// than at its bottom. That keeps the running SP inside the range
/// `sys_process_is_stack` answers for.
///
/// # Panics
///
/// `size` must not exceed [`PS3_PRIMARY_STACK_SIZE`]; the caller
/// rejects a larger declaration before reaching here.
fn primary_stack_base_for(size: u32) -> u64 {
    debug_assert!(
        size as usize <= PS3_PRIMARY_STACK_SIZE,
        "caller rejects a stack larger than the reservation",
    );
    PS3_PRIMARY_STACK_BASE + PS3_PRIMARY_STACK_SIZE as u64 - size as u64
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
