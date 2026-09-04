//! Types the boot stages pass between each other, and the refusals
//! that reject a boot before any stage runs.

use std::time::Duration;

use cellgov_core::Runtime;
use cellgov_time::Budget;

use crate::cli::exit::die;
use crate::game::manifest::TitleManifest;

pub(in crate::game) struct PreparedBoot {
    pub rt: Runtime,
    pub elf_data: Vec<u8>,
    pub timings: StartupTimings,
    pub step_budget: Budget,
    /// Init plans the spawn loader stages for children; the step
    /// loop runs them as the runtime parks each child.
    pub child_init: crate::game::child_init::ChildInitPlans,
    pub authid_source: AuthorityIdSource,
}

/// Where the served program-authority-id came from.
///
/// Renders as the `authid_source=` token of the boot's authority-id
/// witness line, which the cross-runner harness parses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::game) enum AuthorityIdSource {
    /// The SELF identification header carried one.
    SelfHeader,
    /// Raw-ELF input; the host's retail-application fallback stands.
    Fallback,
    /// `CELLGOV_FORCE_SYSTEM_AUTHID` overrode whatever the input said.
    Forced,
}

impl std::fmt::Display for AuthorityIdSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::SelfHeader => "self",
            Self::Fallback => "fallback",
            Self::Forced => "forced",
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(in crate::game) struct StartupTimings {
    pub mem_alloc: Duration,
    pub elf_load: Duration,
    pub hle_bind: Duration,
    pub prx_load: Duration,
}

impl StartupTimings {
    pub fn total(&self) -> Duration {
        self.mem_alloc + self.elf_load + self.hle_bind + self.prx_load
    }
}

pub(in crate::game) struct PrepareOptions<'a> {
    pub title: &'a TitleManifest,
    pub elf_path: &'a str,
    /// Already-decrypted ELF bytes. `prepare` moves them out before
    /// the first stage runs, so this field reads as an empty `Vec`
    /// from inside a stage.
    pub elf_data: Vec<u8>,
    /// Program authority id from the SELF identification header;
    /// `None` (raw-ELF input) keeps the host's retail fallback.
    pub authority_id: Option<u64>,
    /// `ctrl_flags1` from the SELF's plaintext capability header;
    /// `None` for raw-ELF input and for a SELF without the record.
    pub control_flags1: Option<u32>,
    pub firmware_dir: Option<&'a str>,
    /// Store-composed mounts, registered before the manifest's own.
    pub composed_mounts: &'a [crate::composition::ComposedMount],
    /// Which firmware and title version the store composed for this
    /// run; written as the trace stream's header record.
    pub identity: &'a cellgov_compare::RunIdentity,
    pub strict_reserved: bool,
    pub dump_at_pc: Option<u64>,
    pub dump_skip: u32,
    pub print_banner: bool,
    pub profile_pairs: bool,
    pub runtime_max_steps: usize,
    /// Applied after every `module_start` has completed.
    pub patch_bytes: &'a [(u64, u8)],
    pub dump_mem_boot_addrs: &'a [u64],
    /// `--dump-mem-fault` ranges, hex-dumped when a module_start unit
    /// faults or hits `--dump-at-pc`.
    pub dump_mem_fault_ranges: &'a [(u64, u64)],
    pub budget_override: Option<Budget>,
    /// When true, switch runtime mode to `DeterminismCheck` so
    /// per-step `PpuStateHash` records land in the trace buffer.
    pub capture_state_trace: bool,
    /// When true, walk the title ELF's executable PT_LOAD segments
    /// through the PPU decoder before execution and print the gap
    /// report to stderr.
    pub prescan: bool,
    /// Guest argv for the primary thread, `argv[0]` included. Empty
    /// keeps the no-args entry state (r3..r6 = 0).
    pub guest_args: &'a [String],
}

/// Debug toggles captured by both the primary-thread `register_with`
/// and the `set_ppu_factory` closures so children spawned via
/// `sys_ppu_thread_create` inherit them.
#[derive(Debug, Clone, Copy)]
pub(super) struct BootDebugOptions {
    pub dump_at_pc: Option<u64>,
    pub dump_skip: u32,
    pub profile_pairs: bool,
}

impl BootDebugOptions {
    pub(super) fn apply(&self, unit: &mut cellgov_ppu::PpuExecutionUnit) {
        if let Some(pc) = self.dump_at_pc {
            unit.set_break_pc(pc, self.dump_skip);
        }
        if self.profile_pairs {
            unit.set_profile_mode(true);
        }
    }
}

/// Boot configurations that cannot both hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum StrictReservedConflict {
    #[error(
        "boot: --strict-reserved conflicts with title manifest rsx_mirror=true; \
         rsx_mirror requires a writable RSX region but --strict-reserved forces \
         it ReservedStrict. Drop one of the two."
    )]
    RsxMirror,
}

pub(super) fn check_strict_reserved_vs_rsx_mirror(
    strict_reserved: bool,
    rsx_mirror: bool,
) -> Result<(), StrictReservedConflict> {
    if strict_reserved && rsx_mirror {
        return Err(StrictReservedConflict::RsxMirror);
    }
    Ok(())
}

/// Narrow a boot-computed address or size to the `u32` the guest ABI
/// carries it in, dying with `label` when it does not fit.
pub(super) fn u32_or_die(label: &str, value: u64) -> u32 {
    u32::try_from(value)
        .unwrap_or_else(|_| die(&format!("{label}: 0x{value:x} does not fit in u32")))
}

#[cfg(test)]
#[path = "tests/types_tests.rs"]
mod tests;
