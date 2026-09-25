//! `cellgov boot run | bench | bench-once`.

use std::path::PathBuf;

use super::value;
use crate::cli::args::CliArgError;
use cellgov_boot::manifest::CheckpointTrigger;
use cellgov_compare::BootOverrides;

/// Ceiling on `boot bench --runs`. Each run is a whole boot, and the
/// throughput estimator gains nothing past a handful of samples.
const MAX_BENCH_RUNS: usize = 25;

/// A `--runs` count inside the run set's window.
fn bench_runs(s: &str) -> Result<usize, CliArgError> {
    let n: usize = s
        .parse()
        .map_err(|source| CliArgError::CannotParseDecimal {
            context: "runs".to_string(),
            raw: s.to_string(),
            source,
        })?;
    if n == 0 {
        return Err(CliArgError::CountIsZero);
    }
    if n > MAX_BENCH_RUNS {
        return Err(CliArgError::CountTooLarge {
            got: n,
            max: MAX_BENCH_RUNS,
        });
    }
    Ok(n)
}

/// The group [`TitleSelector`] declares; `boot bench --all` joins it,
/// so a sweep and a named title exclude each other the way two named
/// titles do.
const TITLE_SELECTOR_GROUP: &str = "title_selector";

/// Which installed title the boot runs.
#[derive(Debug, Clone, clap::Args)]
#[group(id = TITLE_SELECTOR_GROUP, required = true, multiple = false)]
pub(crate) struct TitleSelector {
    /// Short name from the title registry.
    #[arg(long, value_name = "NAME")]
    pub title: Option<String>,
    /// Content id (serial) from the title registry.
    #[arg(long, value_name = "ID")]
    pub content_id: Option<String>,
    /// A title manifest outside the registry.
    #[arg(long, value_name = "PATH")]
    pub title_manifest: Option<PathBuf>,
}

/// Which firmware and which content version the store composes.
#[derive(Debug, Clone, clap::Args)]
pub(crate) struct BootSelection {
    /// Installed firmware version. Without it, a disc title boots the
    /// firmware its record says it shipped with. Any other title, or a
    /// disc whose record names none, boots the only installed firmware.
    /// With none or several installed, the store refuses.
    #[arg(long, value_name = "VERSION")]
    pub fw: Option<String>,
    /// Installed content version; may be omitted when exactly one is a
    /// candidate.
    #[arg(long, value_name = "base|VERSION")]
    pub game_ver: Option<String>,
    /// A `sys/external` tree outside the store. Marks the run
    /// unmanaged, so it carries no firmware version.
    #[arg(long, value_name = "DIR", conflicts_with = "fw")]
    pub firmware_dir: Option<PathBuf>,
}

/// The help heading [`BootOverrideArgs`] lists its flags under.
///
/// Clap also applies it to each arg a command declares after the
/// flattened struct. Each such arg names its own heading.
pub(super) const BOOT_OVERRIDE_HEADING: &str =
    "Boot overrides (the anchor gates no run that sets one)";

/// The flags that set a run's [`BootOverrides`].
#[derive(Debug, Clone, Default, clap::Args)]
#[command(next_help_heading = BOOT_OVERRIDE_HEADING)]
pub(crate) struct BootOverrideArgs {
    /// Run no firmware module's module_start in the boot process.
    #[arg(long)]
    pub skip_module_start: bool,
    /// Serve the system-class bdj.self program authority id instead of
    /// the one the title's SELF names.
    #[arg(long)]
    pub force_system_authid: bool,
    /// Load the firmware module set at this 64K-aligned base inside the
    /// main region, instead of the first 64K page past the title image.
    /// A spawned child's firmware set loads at the same base.
    #[arg(long, value_name = "HEX", value_parser = value::hex_u64)]
    pub prx_base: Option<u64>,
    /// Run the LLE path of each module_start the boot stubs to CELL_OK.
    #[arg(long)]
    pub disable_module_start_hle_stubs: bool,
}

impl BootOverrideArgs {
    /// The set the run identity carries and the boot applies.
    pub(crate) fn overrides(&self) -> BootOverrides {
        BootOverrides {
            skip_module_start: self.skip_module_start,
            force_system_authid: self.force_system_authid,
            prx_base: self.prx_base,
            disable_module_start_hle_stubs: self.disable_module_start_hle_stubs,
        }
    }
}

/// The flags and values from which [`BootOverrideArgs`] parses `overrides`.
pub(crate) fn override_flags(overrides: &BootOverrides) -> Vec<(&'static str, Option<String>)> {
    let BootOverrides {
        skip_module_start,
        force_system_authid,
        prx_base,
        disable_module_start_hle_stubs,
    } = *overrides;
    let mut out = Vec::new();
    if skip_module_start {
        out.push(("--skip-module-start", None));
    }
    if force_system_authid {
        out.push(("--force-system-authid", None));
    }
    if let Some(base) = prx_base {
        out.push(("--prx-base", Some(format!("0x{base:x}"))));
    }
    if disable_module_start_hle_stubs {
        out.push(("--disable-module-start-hle-stubs", None));
    }
    out
}

/// The outcomes `boot run` has beyond the shared 0-5 contract.
const BOOT_RUN_EXIT_CODES: &str = "Exit codes particular to this command:
  10  the guest faulted
  11  the step cap was reached before the checkpoint
  12  simulated time ran out before a terminal state
  13  the run completed but lost a syscall-wake response
  14  a --save-observation or --save-boot-summary artifact could not
      be produced";

/// The outcomes `boot bench` has beyond the shared 0-5 contract, and
/// how a sweep folds its cells' outcomes into one status.
const BOOT_BENCH_EXIT_CODES: &str = "Exit codes particular to this command:
  15  --strict-perf is set and the run set reaches no throughput
      verdict

With --all, every declared cell of every registry title runs in turn,
one summary line each, and the status is the worst cell's: 3 when a
set broke determinism, 5 when a cell moved off its anchor, 4 when a
cell's boot failed, 15 as above, 1 when a declared cell has no anchor
or no cell ran at all. A cell the registry declares pending, or whose
firmware or dump is not installed, is reported by name and gates
nothing.";

/// `cellgov boot run`
#[derive(Debug, Clone, clap::Args)]
#[command(after_help = BOOT_RUN_EXIT_CODES)]
pub(crate) struct BootRunArgs {
    #[command(flatten)]
    pub selector: TitleSelector,
    #[command(flatten)]
    pub selection: BootSelection,
    /// Boot this executable instead of the one the composition
    /// resolves.
    #[arg(value_name = "ELF")]
    pub elf_path: Option<String>,
    /// Retire at most this many steps.
    #[arg(long, value_name = "N", default_value_t = 100_000)]
    pub max_steps: usize,
    /// Simulated-time budget for the run.
    #[arg(long, value_name = "N")]
    pub budget: Option<u64>,
    /// Emit the binary trace stream.
    #[arg(long)]
    pub trace: bool,
    /// Report per-opcode execution counts.
    #[arg(long)]
    pub profile: bool,
    /// Report the hottest consecutive opcode pairs.
    #[arg(long)]
    pub profile_pairs: bool,
    /// Count the PPU states the run passes through and how many share a
    /// state hash. Holds 32 bytes per dispatched instruction until the run
    /// ends.
    #[arg(long)]
    pub state_hash_census: bool,
    /// Fault on a read of a reserved region instead of serving zeroes.
    #[arg(long)]
    pub strict_reserved: bool,
    /// Scan the title for unimplemented opcodes before booting.
    #[arg(long)]
    pub prescan: bool,
    /// Dump PPU state each time this guest PC retires.
    #[arg(long, value_name = "HEX", value_parser = value::hex_u64)]
    pub dump_at_pc: Option<u64>,
    /// Skip this many `--dump-at-pc` hits before dumping.
    #[arg(long, value_name = "N", default_value_t = 0, requires = "dump_at_pc")]
    pub dump_skip: u32,
    /// Guest addresses to dump once the image is loaded.
    #[arg(long, value_name = "HEX[,HEX]", value_delimiter = ',', value_parser = value::hex_addr)]
    pub dump_mem_boot: Option<Vec<u64>>,
    /// Guest ranges to dump if the run faults.
    #[arg(long, value_name = "HEX[:LEN][,...]", value_delimiter = ',', value_parser = value::dump_mem_fault_range)]
    pub dump_mem_fault: Option<Vec<(u64, u64)>>,
    /// Bytes to overwrite in the loaded image before the first step.
    #[arg(long, value_name = "ADDR=VALUE[,...]", value_delimiter = ',', value_parser = value::patch_byte_pair)]
    pub patch_byte: Option<Vec<(u64, u8)>>,
    /// Write the run's observation JSON here.
    #[arg(long, value_name = "PATH")]
    pub save_observation: Option<String>,
    /// Checkpoint manifest naming the regions the observation covers.
    #[arg(long, value_name = "PATH", requires = "save_observation")]
    pub observation_manifest: Option<String>,
    /// Write the run's boot summary JSON here.
    #[arg(long, value_name = "PATH")]
    pub save_boot_summary: Option<String>,
    /// Write the run's state trace here.
    #[arg(long, value_name = "PATH")]
    pub save_state_trace: Option<String>,
    /// One guest argv entry; repeat for more. Values may spell a flag.
    #[arg(long, value_name = "VALUE", allow_hyphen_values = true, action = clap::ArgAction::Append)]
    pub guest_arg: Vec<String>,
    #[command(flatten)]
    pub overrides: BootOverrideArgs,
}

/// `cellgov boot bench` and `cellgov boot bench-once`.
#[derive(Debug, Clone, clap::Args)]
pub(crate) struct BenchArgs {
    #[command(flatten)]
    pub selector: TitleSelector,
    #[command(flatten)]
    pub selection: BootSelection,
    /// Step cap; defaults to the cap the anchor was recorded at.
    #[arg(long, value_name = "N")]
    pub max_steps: Option<usize>,
    /// Simulated-time budget for the run.
    #[arg(long, value_name = "N")]
    pub budget: Option<u64>,
    /// Stop condition, overriding the manifest's.
    #[arg(long, value_name = "process-exit|first-rsx-write|pc=0xADDR", value_parser = value::checkpoint)]
    pub checkpoint: Option<CheckpointTrigger>,
    /// Scan the title for unimplemented opcodes before booting.
    #[arg(long)]
    pub prescan: bool,
    /// Fault on a read of a reserved region instead of serving zeroes.
    #[arg(long)]
    pub strict_reserved: bool,
    /// One guest argv entry; repeat for more. Values may spell a flag.
    #[arg(long, value_name = "VALUE", allow_hyphen_values = true, action = clap::ArgAction::Append)]
    pub guest_arg: Vec<String>,
    /// Write the run's state trace here. It records a state hash per
    /// step, which makes the run a divergence diagnostic instead of a
    /// throughput measurement.
    #[arg(long, value_name = "PATH")]
    pub save_state_trace: Option<String>,
    /// Index this measurement reports on its `BENCH_RESULT` line. A
    /// run set stamps each of its children.
    #[arg(long, value_name = "N")]
    pub run_index: Option<usize>,
    #[command(flatten)]
    pub overrides: BootOverrideArgs,
}

/// `cellgov boot bench` -- the run set, which alone gates on the
/// anchor.
#[derive(Debug, Clone, clap::Args)]
#[command(after_help = BOOT_BENCH_EXIT_CODES)]
pub(crate) struct BenchGateArgs {
    #[command(flatten)]
    pub bench: BenchArgs,
    // Each arg below names its own heading; see `BOOT_OVERRIDE_HEADING`.
    /// Gate every declared cell of every registry title, one after
    /// another; `--fw` / `--game-ver` narrow the cells.
    #[arg(long, group = TITLE_SELECTOR_GROUP, help_heading = None::<&'static str>)]
    pub all: bool,
    /// Drop the anchor gate for a measurement-only run.
    #[arg(long, help_heading = None::<&'static str>)]
    pub no_anchor_check: bool,
    /// Subprocess measurements to take. With `1` the determinism gate
    /// compares nothing, and the set reports that.
    #[arg(long, value_name = "N", default_value_t = crate::game::BENCH_DEFAULT_RUNS, value_parser = bench_runs, help_heading = None::<&'static str>)]
    pub runs: usize,
    /// Fail when the runs reach no throughput verdict. Use it only on
    /// a host that runs nothing else; elsewhere the spread measures
    /// the host.
    #[arg(long, help_heading = None::<&'static str>)]
    pub strict_perf: bool,
}
