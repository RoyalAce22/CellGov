//! Types the boot stages pass between each other, and the refusals
//! that reject a boot before any stage runs.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use cellgov_core::Runtime;
use cellgov_time::Budget;

use crate::manifest::TitleManifest;
use crate::{BootSink, ChildInitPlans, ComposedMount, DebugTaps, KeyVaultSource};

/// A runtime one `step()` from the title's first instruction.
pub struct PreparedBoot {
    /// The runtime, with the title's primary unit registered and
    /// runnable.
    pub rt: Runtime,
    /// The plaintext title ELF the boot loaded.
    pub elf_data: Vec<u8>,
    /// What each startup stage cost.
    pub timings: StartupTimings,
    /// Retired instructions one `step()` grants.
    pub step_budget: Budget,
    /// Init plans the spawn loader stages for children; the step
    /// loop runs them as the runtime parks each child.
    pub child_init: ChildInitPlans,
    /// Where the served program-authority-id came from.
    pub authid_source: AuthorityIdSource,
}

/// Where the served program-authority-id came from.
///
/// Renders as the `authid_source=` token of the boot's authority-id
/// witness line, which the cross-runner harness parses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityIdSource {
    /// The SELF identification header carried one.
    SelfHeader,
    /// Raw-ELF input; the host's retail-application fallback stands.
    Fallback,
    /// The `force_system_authid` boot override replaced whatever the
    /// input said.
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

/// What each startup stage cost.
#[derive(Debug, Clone, Copy, Default)]
pub struct StartupTimings {
    /// Building the guest address space.
    pub mem_alloc: Duration,
    /// Loading the title image into that address space.
    pub elf_load: Duration,
    /// Parsing the title's import tables.
    pub hle_bind: Duration,
    /// Loading the firmware set and binding imports against it.
    pub prx_load: Duration,
}

impl StartupTimings {
    /// Every stage together.
    pub fn total(&self) -> Duration {
        self.mem_alloc + self.elf_load + self.hle_bind + self.prx_load
    }
}

/// The title the boot runs, and the tree it resolves against.
pub struct TitleOptions<'a> {
    /// The manifest that names the title's source, content and mounts.
    pub manifest: &'a TitleManifest,
    /// Path the executable was read from; names EBOOT siblings and the
    /// default content base.
    pub elf_path: &'a str,
    /// Already-decrypted ELF bytes. [`super::prepare`] moves them out
    /// before the first stage runs, so this field reads as an empty
    /// `Vec` from inside a stage.
    pub elf_data: Vec<u8>,
    /// Program authority id from the SELF identification header;
    /// `None` (raw-ELF input) keeps the host's retail fallback.
    pub authority_id: Option<u64>,
    /// `ctrl_flags1` from the SELF's plaintext capability header;
    /// `None` for raw-ELF input and for a SELF without the record.
    pub control_flags1: Option<u32>,
    /// The `sys/external` tree firmware modules load from.
    pub firmware_dir: Option<&'a str>,
    /// Store-composed mounts, registered before the manifest's own.
    pub composed_mounts: &'a [ComposedMount],
    /// The directories the candidate walk probes for the EBOOT, a
    /// selected update's first: the roots of a mount that declares no
    /// host, and the bases of a `[content]` entry.
    pub eboot_dirs: &'a [PathBuf],
    /// The run's firmware, title version and boot overrides, as the trace header records them.
    ///
    /// The boot reads its overrides from here and nowhere else, so the
    /// header names every override the run applied.
    pub identity: &'a cellgov_compare::RunIdentity,
}

/// How far the boot may run, and what it may change under the guest.
#[derive(Debug, Clone, Copy)]
pub struct ExecutionOptions<'a> {
    /// Retired-instruction cap for the whole run.
    pub runtime_max_steps: usize,
    /// Retired instructions one `step()` grants, overriding the mode
    /// default.
    pub budget_override: Option<Budget>,
    /// Make the reserved regions refuse a read instead of answering
    /// zero.
    pub strict_reserved: bool,
    /// When true, switch runtime mode to `DeterminismCheck` so
    /// per-step `PpuStateHash` records land in the trace buffer, one
    /// per retired instruction.
    pub capture_state_trace: bool,
    /// Guest argv for the primary thread, `argv[0]` included. Empty
    /// keeps the no-args entry state (r3..r6 = 0).
    pub guest_args: &'a [String],
    /// The boot applies these once every `module_start` completes.
    pub patch_bytes: &'a [(u64, u8)],
}

/// What the boot reports about itself, and the debug toggles it applies.
#[derive(Debug, Clone, Copy)]
pub struct DiagnosticOptions<'a> {
    /// Report what the boot loaded, stage by stage. The boot reports
    /// the refusals and the authority-id witness either way.
    pub print_banner: bool,
    /// When true, walk the title ELF's executable PT_LOAD segments
    /// through the PPU decoder before execution and report the gaps.
    pub prescan: bool,
    /// Count adjacent instruction pairs per unit.
    pub profile_pairs: bool,
    /// `--dump-at-pc`: the PC a unit faults with a register dump at.
    /// The fault ends the run.
    pub dump_at_pc: Option<u64>,
    /// How many hits of `dump_at_pc` to pass over first.
    pub dump_skip: u32,
    /// `--dump-mem` addresses, hex-dumped once the boot is up.
    pub dump_mem_boot_addrs: &'a [u64],
    /// `--dump-mem-fault` ranges, hex-dumped when a module_start unit
    /// faults or hits `--dump-at-pc`.
    pub dump_mem_fault_ranges: &'a [(u64, u64)],
}

/// The sink, the vault source and the debug observers, shared for the whole boot.
///
/// All three outlive the options struct. For each child, the spawn
/// loader the boot installs uses the same three:
///
/// - it reports the child's load through the sink,
/// - it opens the vault,
/// - it hands the child's units the observers.
pub struct BootServices {
    /// Where the boot reports what it loaded and what it skipped.
    pub sink: Rc<dyn BootSink>,
    /// Where an SCE-wrapped firmware module or child image gets its
    /// key vault.
    pub keys: Rc<dyn KeyVaultSource>,
    /// The debug observers the boot installs; [`crate::NoTaps`]
    /// installs none.
    pub taps: Rc<dyn DebugTaps>,
}

impl BootServices {
    pub(super) fn sink(&self) -> &dyn BootSink {
        self.sink.as_ref()
    }
}

/// Everything [`super::prepare`] needs to boot a title.
pub struct PrepareOptions<'a> {
    /// What the boot loads.
    pub title: TitleOptions<'a>,
    /// How far it runs and what it may change.
    pub execution: ExecutionOptions<'a>,
    /// What it reports and which debug toggles it applies.
    pub diagnostics: DiagnosticOptions<'a>,
    /// Where it reports, where it gets keys, and which debug observers
    /// it installs.
    pub services: BootServices,
}

/// The debug toggles and the PPU observer for the unit of each PPU thread.
///
/// The primary unit's `register_unit_with` closure and the
/// `set_ppu_factory` closure both capture them, so each thread that
/// `sys_ppu_thread_create` makes inherits them.
#[derive(Clone)]
pub(super) struct BootDebugOptions {
    pub dump_at_pc: Option<u64>,
    pub dump_skip: u32,
    pub profile_pairs: bool,
    pub ppu_tap: Option<Rc<dyn cellgov_ppu::PpuTap>>,
}

impl BootDebugOptions {
    pub(super) fn apply(&self, unit: &mut cellgov_ppu::PpuExecutionUnit) {
        if let Some(pc) = self.dump_at_pc {
            unit.set_break_pc(pc, self.dump_skip);
        }
        if self.profile_pairs {
            unit.set_profile_mode(true);
        }
        if let Some(tap) = &self.ppu_tap {
            unit.set_tap(Rc::clone(tap));
        }
    }
}

/// Boot configurations that cannot both hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StrictReservedConflict {
    /// `--strict-reserved` against a manifest that needs a writable
    /// RSX region.
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

#[cfg(test)]
#[path = "tests/types_tests.rs"]
mod tests;
