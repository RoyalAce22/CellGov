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

/// Reduction policy for findings.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum FuzzReduction {
    /// Preserve the original case only.
    None,
    /// Request a same-fingerprint reduced case.
    OnFinding,
}

/// Common settings for a generated interpreter campaign.
#[derive(Debug, Args)]
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
    /// Report bounded progress after each batch.
    #[arg(long)]
    pub progress: bool,
    /// Maximum detailed findings to retain.
    #[arg(long, default_value_t = 20)]
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
