//! `cellgov boot run | bench | bench-once`.

use std::path::PathBuf;

use super::value;
use crate::game::manifest::CheckpointTrigger;

/// Which installed title the boot runs.
#[derive(Debug, Clone, clap::Args)]
#[group(required = true, multiple = false)]
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
    /// Installed firmware version; may be omitted when exactly one is
    /// a candidate.
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

/// The outcomes `boot run` has beyond the shared 0-5 contract.
const BOOT_RUN_EXIT_CODES: &str = "Exit codes particular to this command:
  10  the guest faulted
  11  the step cap was reached before the checkpoint
  12  simulated time ran out before a terminal state
  13  the run completed but lost a syscall-wake response
  14  a requested artifact could not be written";

/// The outcomes `boot bench` has beyond the shared 0-5 contract.
const BOOT_BENCH_EXIT_CODES: &str = "Exit codes particular to this command:
  15  the pair's wall-time measurement drifted past the gate, or was
      unusable";

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
}

/// `cellgov boot bench` -- the pair, which alone gates on the anchor.
#[derive(Debug, Clone, clap::Args)]
#[command(after_help = BOOT_BENCH_EXIT_CODES)]
pub(crate) struct BenchGateArgs {
    #[command(flatten)]
    pub bench: BenchArgs,
    /// Drop the anchor gate for a measurement-only run.
    #[arg(long)]
    pub no_anchor_check: bool,
}
