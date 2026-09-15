//! `cellgov diff`, `cellgov explore`, and `cellgov scenario`.

use std::path::PathBuf;

use super::boot::{BootSelection, TitleSelector};
use super::value;

/// Human tables or one JSON document.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum OutputFormat {
    #[default]
    Human,
    Json,
}

/// How strictly two observations must agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum CompareModeArg {
    Strict,
    Memory,
    Events,
    Prefix,
}

impl From<CompareModeArg> for cellgov_compare::CompareMode {
    fn from(arg: CompareModeArg) -> Self {
        match arg {
            CompareModeArg::Strict => Self::Strict,
            CompareModeArg::Memory => Self::Memory,
            CompareModeArg::Events => Self::Events,
            CompareModeArg::Prefix => Self::Prefix,
        }
    }
}

/// Which of the shared statuses `diff compare` gives, and for what.
///
/// The command runs its target twice before any comparison, so a
/// disagreement between those two runs is the shared status 3. The 0
/// and 1 rows follow `cellgov_compare::Classification::exits_failure`.
const COMPARE_EXIT_CODES: &str = "Exit codes:
  0   the two runs agreed and no comparison diverged, or a plain run's
      manifest names no scenario this runner has (reported UNSUPPORTED)
  1   neither run produced an observation, a file failed to load or
      save, a baseline flag was given a manifest this runner cannot
      run, the comparison found a divergence, or with
      --observations-dir the baselines disagreed with each other
      (UNSETTLED_ORACLE)
  3   the two runs that had to reproduce each other disagreed, on a
      field or on whether an observation exists at all";

/// The outcomes `diff diverge` has beyond the shared 0-5 contract.
const DIVERGE_EXIT_CODES: &str = "Exit codes particular to this command:
  31  a trace failed to decode, so nothing past the cut was compared";

/// The outcomes `diff zoom` has beyond the shared 0-5 contract.
const ZOOM_EXIT_CODES: &str = "Exit codes particular to this command:
  30  neither window covers the requested step
  31  a zoom trace failed to decode";

/// `cellgov diff ...`
#[derive(Debug, clap::Subcommand)]
pub(crate) enum DiffCommand {
    /// Run a scenario or manifest and compare it against a baseline.
    #[command(after_help = COMPARE_EXIT_CODES)]
    Compare(CompareArgs),
    /// Diff two saved observation JSONs.
    Observations {
        /// First observation JSON.
        #[arg(value_name = "A.json")]
        a: String,
        /// Second observation JSON.
        #[arg(value_name = "B.json")]
        b: String,
    },
    /// Report where two state captures first disagree.
    #[command(after_help = DIVERGE_EXIT_CODES)]
    Diverge {
        /// First state capture.
        #[arg(value_name = "A.state")]
        a: String,
        /// Second state capture.
        #[arg(value_name = "B.state")]
        b: String,
    },
    /// Show one step's register-level diff between two zoom captures.
    #[command(after_help = ZOOM_EXIT_CODES)]
    Zoom {
        /// First zoom capture.
        #[arg(value_name = "A.zoom.state")]
        a: String,
        /// Second zoom capture.
        #[arg(value_name = "B.zoom.state")]
        b: String,
        /// Step to zoom into; `0x` for hex.
        #[arg(value_name = "STEP", value_parser = value::step_count)]
        step: u64,
    },
}

/// `cellgov diff compare`
#[derive(Debug, clap::Args)]
pub(crate) struct CompareArgs {
    /// A scenario name, or a comparison manifest.
    #[arg(value_name = "scenario|manifest.toml")]
    pub target: String,
    /// How strictly the two sides must agree.
    #[arg(long, value_enum, default_value_t = CompareModeArg::Memory)]
    pub mode: CompareModeArg,
    /// Record the scenario's observation here instead of comparing.
    #[arg(long, value_name = "PATH", conflicts_with_all = ["against_baseline", "observations_dir", "mode", "format"])]
    pub save_baseline: Option<String>,
    /// Compare the scenario against this recorded baseline.
    #[arg(long, value_name = "PATH", conflicts_with = "observations_dir")]
    pub against_baseline: Option<String>,
    /// Compare every observation in this directory; manifest targets
    /// only.
    #[arg(long, value_name = "DIR")]
    pub observations_dir: Option<String>,
}

/// `cellgov explore`
#[derive(Debug, clap::Args)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
pub(crate) struct ExploreArgs {
    /// Scenario to explore.
    #[arg(value_name = "SCENARIO", required = true)]
    pub scenario: Option<String>,
    #[command(subcommand)]
    pub command: Option<ExploreCommand>,
}

/// `cellgov explore ...`
#[derive(Debug, clap::Subcommand)]
pub(crate) enum ExploreCommand {
    /// Explore an LV2-driven micro-test.
    Micro {
        /// Micro-test name.
        #[arg(value_name = "NAME")]
        name: String,
        /// Compare each schedule against the observations here.
        #[arg(long, value_name = "DIR")]
        observations_dir: Option<PathBuf>,
    },
    /// Explore a window of a composed title boot.
    #[command(after_help = EXPLORE_TITLE_EXIT_CODES)]
    Title(Box<ExploreTitleArgs>),
}

/// The group the two window-start flags join, so naming both is a usage
/// error rather than a silent precedence.
const WINDOW_START_GROUP: &str = "window_start";

/// The outcomes `explore title` has beyond the shared 0-5 contract.
const EXPLORE_TITLE_EXIT_CODES: &str = "Exit codes particular to this command:
  20  the model refused a schedule it was asked to explore: a refused
      commit, or a refused step. The cell's own first-rsx-write
      checkpoint is not one of them.
  21  the window never opened: the boot reached a terminal state, a cap
      or a refusal before the start condition

A schedule-sensitive window -- two schedules that both ran themselves
out committed different memory -- takes the shared status 1. A cap the
caller set, and a window whose units all blocked, report inconclusive
and exit 0.";

/// `cellgov explore title`
#[derive(Debug, clap::Args)]
pub(crate) struct ExploreTitleArgs {
    #[command(flatten)]
    pub selector: TitleSelector,
    #[command(flatten)]
    pub selection: BootSelection,
    /// Retired-instruction cap for the whole boot, the window
    /// included; defaults to the cap the cell's anchor was recorded at.
    #[arg(long, value_name = "N")]
    pub max_steps: Option<usize>,
    /// Explore at most this many alternate schedules.
    #[arg(long, value_name = "N", default_value_t = cellgov_explore::config::DEFAULT_MAX_SCHEDULES)]
    pub max_schedules: usize,
    /// Take at most this many runtime steps per replayed schedule.
    #[arg(long, value_name = "N", default_value_t = cellgov_explore::config::DEFAULT_MAX_STEPS_PER_RUN)]
    pub max_steps_per_run: usize,
    /// Open the window after this many runtime steps.
    /// Without it and without --start-pc, the window opens at the
    /// first step two units are runnable at.
    #[arg(long, value_name = "N", group = WINDOW_START_GROUP)]
    pub start_step: Option<usize>,
    /// Open the window once a step yields at this guest PC. A PC
    /// reached inside a batch never matches.
    #[arg(long, value_name = "HEX", value_parser = value::hex_u64, group = WINDOW_START_GROUP)]
    pub start_pc: Option<u64>,
}

/// `cellgov scenario ...`
#[derive(Debug, clap::Subcommand)]
pub(crate) enum ScenarioCommand {
    /// Name every synthetic scenario.
    List,
    /// Run one scenario and print its report.
    Run {
        /// Scenario name.
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// Run one scenario and print every trace record.
    Dump {
        /// Scenario name.
        #[arg(value_name = "NAME")]
        name: String,
    },
}
