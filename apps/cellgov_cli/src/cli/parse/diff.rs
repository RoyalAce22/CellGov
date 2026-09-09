//! `cellgov diff`, `cellgov explore`, and `cellgov scenario`.

use std::path::PathBuf;

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
  1   a run produced no observation, a file failed to load or save,
      a baseline flag was given a manifest this runner cannot run,
      the comparison found a divergence, or with --observations-dir
      the baselines disagreed with each other (UNSETTLED_ORACLE)
  3   the two runs that had to reproduce each other disagreed";

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
    pub micro: Option<ExploreCommand>,
}

/// `cellgov explore micro`
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
