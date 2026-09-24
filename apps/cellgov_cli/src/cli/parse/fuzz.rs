//! Declarative arguments for `cellgov dev fuzz`.

use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};

use super::value;

/// Fuzz command family.
#[derive(Debug, Args)]
pub(crate) struct FuzzArgs {
    #[command(subcommand)]
    pub command: FuzzCommand,
}

/// Interpreter campaign or decoder sweep selected by the operator.
#[derive(Debug, Subcommand)]
pub(crate) enum FuzzCommand {
    /// Check generated PPU instructions.
    PpuInstruction(FuzzCampaignArgs),
    /// Check generated PPU sequences.
    PpuSequence(FuzzCampaignArgs),
    /// Check generated SPU instructions.
    SpuInstruction(FuzzCampaignArgs),
    /// Check generated SPU sequences.
    SpuSequence(FuzzCampaignArgs),
    /// Enumerate interpreter-owned semantic classes.
    Semantic(FuzzSemanticArgs),
    /// Scan raw instruction words through one decoder.
    Raw(FuzzRawArgs),
    /// Classify every PPU word in a range against the decoder, the encoder and the gap tables.
    Census(FuzzCensusArgs),
    /// Merge census shards that tile one word interval into one result.
    CensusMerge(FuzzCensusMergeArgs),
    /// Replay a versioned finding artifact against its original engine.
    Replay(FuzzReplayArgs),
    /// Run repeated trials of one engine at one budget and store their distributions.
    Evaluate(FuzzEvaluateArgs),
    /// Rank a stored evaluation against a baseline at equal budgets.
    Compare(FuzzCompareArgs),
    /// Run the bounded smoke set and hold every finding to a promoted regression.
    Smoke(FuzzSmokeArgs),
    /// Promote a minimized finding artifact into a regression directory as open.
    Promote(FuzzPromoteArgs),
}

/// Input construction strategy supported by instruction and sequence engines.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum FuzzStrategy {
    /// Use interpreter-owned typed descriptors.
    Structured,
    /// Feed complete 32-bit words to the decoder.
    RawWords,
}

/// Selection for checks an engine performs.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum FuzzCheck {
    /// Run every check declared by the selected engine.
    All,
    /// Request an invariant-only run.
    Invariant,
    /// Request a metamorphic-only run.
    Metamorphic,
    /// Request an internal-path-only run.
    Paths,
}

/// Whether to reduce retained findings.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum FuzzReduction {
    /// Preserve the original case only.
    None,
    /// Reduce each retained finding to a smaller same-fingerprint case.
    OnFinding,
}

/// How a reducer chooses among reproducing candidates.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum FuzzReductionPolicy {
    /// Accept the lowest-ordered reproducing candidate of each round.
    Deterministic,
    /// Accept the first reproducing candidate the driver reports.
    Greedy,
}

impl From<FuzzStrategy> for cellgov_fuzz::GenerationStrategy {
    fn from(strategy: FuzzStrategy) -> Self {
        match strategy {
            FuzzStrategy::Structured => Self::Structured,
            FuzzStrategy::RawWords => Self::RawWords,
        }
    }
}

impl From<FuzzReductionPolicy> for cellgov_fuzz::reduce::ReductionPolicy {
    fn from(policy: FuzzReductionPolicy) -> Self {
        match policy {
            FuzzReductionPolicy::Deterministic => Self::Deterministic,
            FuzzReductionPolicy::Greedy => Self::Greedy,
        }
    }
}

/// The outcomes a generated campaign has beyond the shared 0-5 contract.
pub(crate) const CAMPAIGN_EXIT_CODES: &str = "Exit codes particular to this command:
  0   every scheduled case ran, with or without findings; the summary
      line's outcome field says which, and each finding line names a
      replay
  1   the shared failed-operation status when --reference could not be
      read or disagreed with the interpreter, or a worker failed
  10  the range ended on --deadline-ms or --cancel-after before every
      case ran, findings or not
  11  every case ran and none was eligible for its check
  12  the engine failed inside the harness rather than the target
  13  a finding's artifact could not be stored; its evidence was printed
  14  a retained finding's reduction failed; its original case is stored
  141 stdout was closed by a downstream reader";

/// The outcomes `dev fuzz semantic` has beyond the shared 0-5 contract.
pub(crate) const SEMANTIC_EXIT_CODES: &str = "Exit codes particular to this command:
  1   a descriptor registry disagreed with its interpreter
  141 stdout was closed by a downstream reader";

/// The outcomes `dev fuzz raw` has beyond the shared 0-5 contract.
pub(crate) const RAW_EXIT_CODES: &str = "Exit codes particular to this command:
  1   the decoder panicked on at least one word; also the shared
      failed-operation status when --output could not be written or the
      sweep failed before its words were classified
  10  the scan ended on --deadline-ms or --cancel-after before every
      word ran, with no panic
  141 stdout was closed by a downstream reader";

/// The outcomes `dev fuzz census` has beyond the shared 0-5 contract.
pub(crate) const CENSUS_EXIT_CODES: &str = "Exit codes particular to this command:
  1   a word broke a census property: a decoded word did not round-trip
      through the encoder, a rejection named a mnemonic outside the gap
      tables, a word under primary opcode 0 decoded, or the decoder
      panicked; also the shared failed-operation status when --output
      could not be written or a worker failed
  141 stdout was closed by a downstream reader";

/// The outcomes `dev fuzz census-merge` has beyond the shared 0-5 contract.
pub(crate) const CENSUS_MERGE_EXIT_CODES: &str = "Exit codes particular to this command:
  1   the merged census holds a finding; also the shared failed-operation
      status when an input could not be read or parsed, carries a foreign
      schema version or inconsistent counts, the inputs do not tile one
      contiguous interval, --full found a shard missing at either end,
      or --output could not be written
  141 stdout was closed by a downstream reader";

/// The outcomes `dev fuzz replay` has beyond the shared 0-5 contract.
pub(crate) const REPLAY_EXIT_CODES: &str = "Exit codes particular to this command:
  1   the stored finding reproduced; also the shared failed-operation
      status when --artifact could not be read or its independent
      reference no longer matched
  12  the engine failed inside the harness rather than the target
  15  the stored case no longer reproduces its finding
  141 stdout was closed by a downstream reader";

/// The outcomes `dev fuzz evaluate` has beyond the shared 0-5 contract.
pub(crate) const EVALUATE_EXIT_CODES: &str = "Exit codes particular to this command:
  1   the shared failed-operation status when --output could not be
      written or --baseline could not be read
  12  a trial's engine failed inside the harness; the results were
      still written
  16  the candidate regressed a validity or coverage metric against
      --baseline
  141 stdout was closed by a downstream reader";

/// The outcomes `dev fuzz compare` has beyond the shared 0-5 contract.
pub(crate) const COMPARE_EXIT_CODES: &str = "Exit codes particular to this command:
  1   the shared failed-operation status when a stored evaluation could
      not be read or is not a complete evaluation
  16  the candidate regressed a validity or coverage metric against the
      baseline
  141 stdout was closed by a downstream reader";

/// Common settings for a generated interpreter campaign.
#[derive(Debug, Args)]
#[command(after_help = CAMPAIGN_EXIT_CODES)]
#[command(group = clap::ArgGroup::new("case-selection").args(["replay_case", "first"]))]
pub(crate) struct FuzzCampaignArgs {
    /// Serialized generator version required for exact replay.
    #[arg(long, default_value_t = cellgov_fuzz::CAMPAIGN_VERSION.0)]
    pub campaign_version: u32,
    /// Master deterministic seed.
    #[arg(long, default_value_t = 1)]
    pub seed: u64,
    /// First case index in a bounded range.
    #[arg(long, default_value_t = 0, conflicts_with = "replay_case")]
    pub first: u64,
    /// Number of case indices to consider.
    #[arg(long, default_value_t = 100, conflicts_with = "replay_case")]
    pub count: u64,
    /// Run exactly this original case index.
    #[arg(long, conflicts_with_all = ["first", "count", "shard", "shards"])]
    pub replay_case: Option<u64>,
    /// Zero-based deterministic shard index.
    #[arg(long, default_value_t = 0)]
    pub shard: u32,
    /// Number of deterministic shards.
    #[arg(long, default_value_t = 1)]
    pub shards: u32,
    /// Host worker count; defaults to available parallelism.
    #[arg(long)]
    pub workers: Option<usize>,
    /// Stop at this deterministic offset in the requested range.
    #[arg(long)]
    pub cancel_after: Option<u64>,
    /// Host deadline in milliseconds. The engine checks it between bounded batches.
    #[arg(long)]
    pub deadline_ms: Option<u64>,
    /// Show a progress bar on stderr; threshold lines when stderr is not a terminal.
    #[arg(long)]
    pub progress: bool,
    /// Maximum detailed findings to retain.
    #[arg(long, default_value_t = cellgov_fuzz::DEFAULT_MAX_FINDINGS)]
    pub finding_limit: u32,
    /// Number of words generated for each sequence case.
    #[arg(long)]
    pub sequence_words: Option<u32>,
    /// Generate typed instruction forms or raw decoder words.
    #[arg(long, value_enum, default_value_t = FuzzStrategy::Structured)]
    pub strategy: FuzzStrategy,
    /// Select checks from the chosen engine.
    #[arg(long, value_enum, default_value_t = FuzzCheck::All)]
    pub check: FuzzCheck,
    /// Optional versioned independent PPU or SPU reference artifact.
    #[arg(long, value_name = "PATH")]
    pub reference: Option<PathBuf>,
    /// Request reduction of retained findings.
    #[arg(long, value_enum, default_value_t = FuzzReduction::None)]
    pub reduction: FuzzReduction,
    /// Candidate selection policy for reduction.
    #[arg(long, value_enum, default_value_t = FuzzReductionPolicy::Deterministic)]
    pub reduction_policy: FuzzReductionPolicy,
    /// Maximum candidate evaluations spent on each finding.
    #[arg(long, default_value_t = cellgov_fuzz::reduce::DEFAULT_REDUCTION_BUDGET)]
    pub reduction_budget: u64,
    /// Directory that receives one versioned artifact per retained finding.
    #[arg(long, value_name = "DIR", default_value = "target/fuzz-findings")]
    pub artifacts_dir: PathBuf,
}

/// Interpreter set for descriptor-derived enumeration.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum FuzzSemanticTarget {
    /// Enumerate both interpreter registries.
    Both,
    /// Enumerate only PPU descriptors.
    Ppu,
    /// Enumerate only SPU descriptors.
    Spu,
}

/// Settings for a descriptor-derived semantic enumeration.
#[derive(Debug, Args)]
#[command(after_help = SEMANTIC_EXIT_CODES)]
pub(crate) struct FuzzSemanticArgs {
    /// Interpreter registry to enumerate.
    #[arg(value_enum, default_value_t = FuzzSemanticTarget::Both)]
    pub target: FuzzSemanticTarget,
    /// Report a line after each selected interpreter.
    #[arg(long)]
    pub progress: bool,
}

/// Decoder selected for a raw-word scan.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum FuzzRawDecoder {
    /// PowerPC decoder.
    Ppu,
    /// Synergistic-processor decoder.
    Spu,
}

/// Settings for a bounded or explicitly sharded raw decoder scan.
#[derive(Debug, Args)]
#[command(after_help = RAW_EXIT_CODES)]
#[command(group = clap::ArgGroup::new("scan-scope").required(true).args(["full", "count"]))]
pub(crate) struct FuzzRawArgs {
    /// Decoder whose outcomes to classify.
    #[arg(value_enum)]
    pub decoder: FuzzRawDecoder,
    /// Scan one shard of the full 32-bit word space.
    #[arg(long, conflicts_with = "count")]
    pub full: bool,
    /// First bounded word, in hexadecimal.
    #[arg(long, conflicts_with = "full", value_parser = value::hex_u32)]
    pub start: Option<u32>,
    /// Bounded word count.
    #[arg(long)]
    pub count: Option<u64>,
    /// Zero-based full-domain shard index.
    #[arg(long, conflicts_with = "count")]
    pub shard: Option<u32>,
    /// Number of full-domain shards.
    #[arg(long, conflicts_with = "count")]
    pub shards: Option<u32>,
    /// Maximum words per finite-domain batch.
    #[arg(long, default_value_t = cellgov_fuzz::raw_decode::MAX_RAW_DECODE_CHUNK)]
    pub chunk_size: usize,
    /// Host worker count; defaults to available parallelism.
    #[arg(long)]
    pub workers: Option<usize>,
    /// Stop at this deterministic word offset.
    #[arg(long)]
    pub cancel_after: Option<u64>,
    /// Host deadline in milliseconds. The engine checks it between bounded batches.
    #[arg(long)]
    pub deadline_ms: Option<u64>,
    /// Report bounded progress after each batch.
    #[arg(long)]
    pub progress: bool,
    /// Maximum detailed target panic records.
    #[arg(long, default_value_t = cellgov_fuzz::raw_decode::MAX_RAW_DECODE_PANIC_SAMPLES)]
    pub finding_limit: usize,
    /// Write the versioned JSON result here.
    #[arg(long, value_name = "PATH")]
    pub output: Option<PathBuf>,
    /// Request a same-class reduced case.
    #[arg(long, value_enum, default_value_t = FuzzReduction::None)]
    pub reduction: FuzzReduction,
}

/// Settings for a bounded or sharded PPU decoder census.
///
/// The census decodes and re-encodes every word in the range and checks
/// each rejection against the gap tables. The whole 32-bit space takes
/// about two minutes on one core and splits evenly across --workers and
/// across --shards.
#[derive(Debug, Args)]
#[command(after_help = CENSUS_EXIT_CODES)]
#[command(group = clap::ArgGroup::new("census-scope").required(true).args(["full", "count"]))]
pub(crate) struct FuzzCensusArgs {
    /// Cover one shard of the full 32-bit word space.
    #[arg(long, conflicts_with = "count")]
    pub full: bool,
    /// First bounded word, in hexadecimal.
    #[arg(long, conflicts_with = "full", value_parser = value::hex_u32)]
    pub start: Option<u32>,
    /// Bounded word count.
    #[arg(long)]
    pub count: Option<u64>,
    /// Zero-based full-domain shard index.
    #[arg(long, conflicts_with = "count")]
    pub shard: Option<u32>,
    /// Number of full-domain shards.
    #[arg(long, conflicts_with = "count")]
    pub shards: Option<u32>,
    /// Host worker count; defaults to available parallelism.
    #[arg(long)]
    pub workers: Option<usize>,
    /// Report progress after each bounded batch.
    #[arg(long)]
    pub progress: bool,
    /// Write the versioned JSON result here.
    #[arg(long, value_name = "PATH")]
    pub output: Option<PathBuf>,
}

/// Settings for merging census shards.
#[derive(Debug, Args)]
#[command(after_help = CENSUS_MERGE_EXIT_CODES)]
pub(crate) struct FuzzCensusMergeArgs {
    /// Versioned census results that tile one contiguous word interval.
    #[arg(required = true, value_name = "PATH")]
    pub inputs: Vec<PathBuf>,
    /// Refuse a merged interval that does not cover the whole 32-bit word space.
    #[arg(long)]
    pub full: bool,
    /// Write the merged JSON result here.
    #[arg(long, value_name = "PATH")]
    pub output: Option<PathBuf>,
}

/// Exact replay of one stored finding artifact.
#[derive(Debug, Args)]
#[command(after_help = REPLAY_EXIT_CODES)]
pub(crate) struct FuzzReplayArgs {
    /// Versioned finding JSON to replay.
    #[arg(long, value_name = "PATH")]
    pub artifact: PathBuf,
    /// Replay the recorded reduced case instead of the original.
    #[arg(long)]
    pub reduced: bool,
}

/// Engine under repeated-trial evaluation.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum FuzzEvaluateEngine {
    /// Generated PPU instructions.
    PpuInstruction,
    /// Generated PPU sequences.
    PpuSequence,
    /// Generated SPU instructions.
    SpuInstruction,
    /// Generated SPU sequences.
    SpuSequence,
}

impl From<FuzzEvaluateEngine> for cellgov_fuzz::FuzzTarget {
    fn from(engine: FuzzEvaluateEngine) -> Self {
        match engine {
            FuzzEvaluateEngine::PpuInstruction => Self::PpuInstruction,
            FuzzEvaluateEngine::PpuSequence => Self::PpuSequence,
            FuzzEvaluateEngine::SpuInstruction => Self::SpuInstruction,
            FuzzEvaluateEngine::SpuSequence => Self::SpuSequence,
        }
    }
}

/// Settings for repeated equal-budget trials of one engine.
#[derive(Debug, Args)]
#[command(after_help = EVALUATE_EXIT_CODES)]
pub(crate) struct FuzzEvaluateArgs {
    /// Engine every trial runs.
    #[arg(value_enum)]
    pub engine: FuzzEvaluateEngine,
    /// Number of trials; each runs one consecutive seed from --first-seed.
    #[arg(long, default_value_t = 10)]
    pub trials: u32,
    /// Seed of the first trial.
    #[arg(long, default_value_t = 1)]
    pub first_seed: u64,
    /// Case indices every trial considers.
    #[arg(long, default_value_t = 100)]
    pub cases: u64,
    /// Number of words generated for each sequence case.
    #[arg(long)]
    pub sequence_words: Option<u32>,
    /// Generate typed instruction forms or raw decoder words.
    #[arg(long, value_enum, default_value_t = FuzzStrategy::Structured)]
    pub strategy: FuzzStrategy,
    /// Maximum detailed findings each trial retains.
    #[arg(long, default_value_t = cellgov_fuzz::DEFAULT_MAX_FINDINGS)]
    pub finding_limit: u32,
    /// Reduce each trial's retained findings and record the cost.
    #[arg(long, value_enum, default_value_t = FuzzReduction::None)]
    pub reduction: FuzzReduction,
    /// Candidate selection policy for reduction.
    #[arg(long, value_enum, default_value_t = FuzzReductionPolicy::Deterministic)]
    pub reduction_policy: FuzzReductionPolicy,
    /// Maximum candidate evaluations spent on each finding.
    #[arg(long, default_value_t = cellgov_fuzz::reduce::DEFAULT_REDUCTION_BUDGET)]
    pub reduction_budget: u64,
    /// Trials run at once; defaults to available parallelism.
    #[arg(long)]
    pub workers: Option<usize>,
    /// Report a line after each trial.
    #[arg(long)]
    pub progress: bool,
    /// Write the versioned JSON results here.
    #[arg(long, value_name = "PATH")]
    pub output: PathBuf,
    /// Stored evaluation to rank these results against.
    #[arg(long, value_name = "PATH")]
    pub baseline: Option<PathBuf>,
}

/// The outcomes `dev fuzz smoke` has beyond the shared 0-5 contract.
pub(crate) const SMOKE_EXIT_CODES: &str = "Exit codes particular to this command:
  1   a campaign retained a finding no promoted regression covers; its
      artifact names the exact replay. Also the shared failed-operation
      status when --regressions could not be loaded
  12  an engine failed inside the harness rather than the target
  13  a finding's artifact could not be stored; its evidence was printed
  14  a retained finding's reduction failed; its original case is stored
  17  a campaign reached less than its coverage floor
  141 stdout was closed by a downstream reader";

/// Settings for the bounded smoke set.
#[derive(Debug, Args)]
#[command(after_help = SMOKE_EXIT_CODES)]
pub(crate) struct FuzzSmokeArgs {
    /// Directory that receives one minimized artifact per retained finding.
    #[arg(long, value_name = "DIR", default_value = "target/fuzz-smoke")]
    pub artifacts_dir: PathBuf,
    /// Regression directory whose open entries cover known findings.
    #[arg(long, value_name = "DIR")]
    pub regressions: Option<PathBuf>,
    /// Maximum candidate evaluations spent reducing each finding.
    #[arg(long, default_value_t = cellgov_fuzz::reduce::DEFAULT_REDUCTION_BUDGET)]
    pub reduction_budget: u64,
    /// Report a line after each campaign.
    #[arg(long)]
    pub progress: bool,
}

/// The outcomes `dev fuzz promote` has beyond the shared 0-5 contract.
pub(crate) const PROMOTE_EXIT_CODES: &str = "Exit codes particular to this command:
  1   the artifact could not be read, the regression directory could not
      be loaded or written, the artifact is not minimized, or the finding
      or name is already promoted
  141 stdout was closed by a downstream reader";

/// Build profile a promoted finding reproduces in.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum FuzzRegressionProfile {
    /// Debug and release builds alike.
    Both,
    /// Debug builds only.
    Debug,
    /// Release builds only.
    Release,
}

/// Settings for the promotion of one finding.
#[derive(Debug, Args)]
#[command(after_help = PROMOTE_EXIT_CODES)]
pub(crate) struct FuzzPromoteArgs {
    /// Minimized finding artifact to promote.
    #[arg(long, value_name = "PATH")]
    pub artifact: PathBuf,
    /// Regression directory that receives the artifact and the manifest entry.
    #[arg(long, value_name = "DIR")]
    pub regressions: PathBuf,
    /// Name of the regression: lowercase letters, digits, '-' and '_'.
    #[arg(long)]
    pub name: String,
    /// What the finding is, in one sentence.
    #[arg(long)]
    pub summary: String,
    /// Build profile the finding reproduces in.
    #[arg(long, value_enum, default_value_t = FuzzRegressionProfile::Both)]
    pub profile: FuzzRegressionProfile,
}

/// Two stored evaluations to rank.
#[derive(Debug, Args)]
#[command(after_help = COMPARE_EXIT_CODES)]
pub(crate) struct FuzzCompareArgs {
    /// Stored evaluation to rank the candidate against.
    #[arg(long, value_name = "PATH")]
    pub baseline: PathBuf,
    /// Stored evaluation under review.
    #[arg(long, value_name = "PATH")]
    pub candidate: PathBuf,
}
