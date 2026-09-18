//! `cellgov dev ...` -- maintainer tooling.

use std::path::PathBuf;

use super::boot::BootSelection;
use super::value;
use crate::cli::args::CliArgError;
use crate::cli::exit::SCE_INPUT_USAGE_NOTE;

/// The outcomes `dev disasm` has beyond the shared 0-5 contract.
const DISASM_EXIT_CODES: &str = "Exit codes particular to this command:
  20   at least one word decoded to no instruction
  141  stdout was closed by a downstream reader";

/// Hard cap on `dev disasm --count`. PPC instructions are 4 bytes, so
/// 1<<16 lines covers a 256 KB code region.
pub(crate) const MAX_DISASM_COUNT: usize = 1 << 16;

/// `cellgov dev ...`
#[derive(Debug, clap::Subcommand)]
pub(crate) enum DevCommand {
    /// Disassemble a guest ELF at a virtual address.
    Disasm(DisasmArgs),
    /// Print a PRX or executable's import table.
    PrxImports(PrxImportsArgs),
    /// Print the OPD-derived function map for an ELF or PRX.
    Funcs(FuncsArgs),
    /// Locate the syscall dispatch table in a decrypted LV2 kernel.
    Lv2Discover(Lv2DiscoverArgs),
    /// Emit the LV2 census rows for one firmware version.
    Lv2Census(Lv2CensusArgs),
    /// Decrypt one installed firmware's stored LV2 kernel.
    #[cfg(feature = "decrypt")]
    Lv2Extract(Lv2ExtractArgs),
    /// Extract the PPU syscall caller census from installed firmware.
    #[cfg(feature = "decrypt")]
    CallerCensus(CallerCensusArgs),
    /// Answer which HLE call wrote a guest address, from a trace.
    Rpcs3Attribute(Rpcs3AttributeArgs),
    /// Regenerate a title's cross-runner fixture directory.
    FixtureGen(Box<FixtureGenArgs>),
    /// Regenerate the title documents from the registry and fixtures.
    TitlesGen(TitlesGenArgs),
    /// Regenerate `docs/cli.md` from this command tree.
    CliGen(CliGenArgs),
    /// Print a shell completion script for this command tree.
    Completions(CompletionsArgs),
    /// Emit a title-manifest stub from an install record.
    GenManifest(GenManifestArgs),
    /// Re-measure titles and rewrite their committed anchors.
    RecordAnchors(RecordAnchorsArgs),
}

#[derive(Debug, clap::Args)]
pub(crate) struct Lv2CensusArgs {
    /// Decrypted kernel ELF, or a SELF the configured vault can open.
    #[arg(value_name = "ELF")]
    pub path: PathBuf,
    /// Select the firmware version from `pup.tsv`.
    #[arg(long, value_name = "VERSION")]
    pub fw: String,
    /// Use the source PUP's SHA-256 from `pup.tsv`.
    #[arg(long, value_name = "SHA256")]
    pub pup_sha256: String,
    /// Write the archive rows to this directory.
    #[arg(long, value_name = "DIR", default_value = "docs/lv2")]
    pub output_dir: PathBuf,
    /// Replace all rows for this firmware with rows from the selected PUP.
    #[arg(long)]
    pub replace_version: bool,
}

#[cfg(feature = "decrypt")]
#[derive(Debug, clap::Args)]
#[command(group = clap::ArgGroup::new("caller-census-scope")
    .required(true)
    .args(["all", "fw"]))]
pub(crate) struct CallerCensusArgs {
    /// Scan every installed firmware, with title firmware first.
    #[arg(long)]
    pub all: bool,
    /// Scan one installed firmware version.
    #[arg(long, value_name = "VERSION")]
    pub fw: Option<String>,
    /// Write the three archive tables to this directory.
    #[arg(long, value_name = "DIR", default_value = "docs/lv2")]
    pub output_dir: PathBuf,
}

/// `cellgov dev lv2-extract`
#[cfg(feature = "decrypt")]
#[derive(Debug, clap::Args)]
#[command(after_help = "Firmware selection:
  --fw names an installed version. Without it, the only installed
  version is selected; none or several installed versions are refused.")]
pub(crate) struct Lv2ExtractArgs {
    /// Installed firmware version. Omit when the store holds exactly one.
    #[arg(long, value_name = "VERSION")]
    pub fw: Option<String>,
    /// Directory that receives the plaintext kernel ELF.
    #[arg(long, value_name = "DIR", required = true)]
    pub output_dir: PathBuf,
}

/// `cellgov dev disasm`
#[derive(Debug, clap::Args)]
#[command(after_help = format!("{SCE_INPUT_USAGE_NOTE}

{DISASM_EXIT_CODES}"))]
pub(crate) struct DisasmArgs {
    /// Guest ELF, PRX, or SELF.
    #[arg(value_name = "ELF")]
    pub elf_path: String,
    /// Virtual address to start at; must be 4-byte aligned.
    #[arg(long, value_name = "HEX", value_parser = value::hex_u64)]
    pub vaddr: u64,
    /// Instruction count.
    #[arg(long, value_name = "N", default_value_t = 16, value_parser = disasm_count)]
    pub count: usize,
    /// Build the OPD function map and annotate branch targets.
    #[arg(long)]
    pub symbolize: bool,
}

/// A `--count` inside the disassembler's window.
fn disasm_count(s: &str) -> Result<usize, CliArgError> {
    let n: usize = s
        .parse()
        .map_err(|source| CliArgError::CannotParseDecimal {
            context: "count".to_string(),
            raw: s.to_string(),
            source,
        })?;
    if n == 0 {
        return Err(CliArgError::CountIsZero);
    }
    if n > MAX_DISASM_COUNT {
        return Err(CliArgError::CountTooLarge {
            got: n,
            max: MAX_DISASM_COUNT,
        });
    }
    Ok(n)
}

/// `cellgov dev prx-imports`
#[derive(Debug, clap::Args)]
#[command(after_help = SCE_INPUT_USAGE_NOTE)]
pub(crate) struct PrxImportsArgs {
    /// A `.prx`, `.sprx`, or title executable.
    #[arg(value_name = "PATH")]
    pub path: PathBuf,
    /// Show only the import whose stub sits at this file-relative
    /// address.
    #[arg(long, value_name = "HEX", value_parser = value::hex_u32)]
    pub at: Option<u32>,
    /// Show only imports from this module.
    #[arg(long, value_name = "NAME")]
    pub module: Option<String>,
    /// Write the decrypted plaintext ELF here.
    #[arg(long, value_name = "PATH")]
    pub save_elf: Option<PathBuf>,
}

impl PrxImportsArgs {
    /// Whether the listing covers the whole import table.
    ///
    /// The caller reports a file-wide tally only for an unfiltered run.
    pub fn is_unfiltered(&self) -> bool {
        self.at.is_none() && self.module.is_none()
    }
}

/// `cellgov dev funcs`
#[derive(Debug, clap::Args)]
#[command(after_help = SCE_INPUT_USAGE_NOTE)]
pub(crate) struct FuncsArgs {
    /// Guest ELF, PRX, or SELF.
    #[arg(value_name = "ELF")]
    pub path: String,
    /// Emit the map as JSON instead of a table.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, clap::Args)]
#[command(after_help = SCE_INPUT_USAGE_NOTE)]
pub(crate) struct Lv2DiscoverArgs {
    /// Decrypted LV2 kernel ELF, or an SCE wrapper in a decrypt build.
    #[arg(value_name = "ELF")]
    pub path: String,
}

/// `cellgov dev rpcs3-attribute`
#[derive(Debug, clap::Args)]
#[command(group = clap::ArgGroup::new("attribute-query").required(true).multiple(true))]
pub(crate) struct Rpcs3AttributeArgs {
    /// The HLE trace to read.
    #[arg(long, value_name = "PATH")]
    pub trace: PathBuf,
    /// Report the calls that wrote this guest address.
    #[arg(long, value_name = "HEX", group = "attribute-query", value_parser = value::hex_u64)]
    pub addr: Option<u64>,
    /// Bytes covered from `--addr`, hex like the address (default 1).
    #[arg(long, value_name = "HEX", requires = "addr", value_parser = value::hex_u64)]
    pub len: Option<u64>,
    /// List every call in the trace.
    #[arg(long, group = "attribute-query")]
    pub list: bool,
    /// Rank the calls by how much they wrote.
    #[arg(long, group = "attribute-query")]
    pub ranked: bool,
    /// Report only calls whose name contains this substring.
    #[arg(long, value_name = "SUBSTR", group = "attribute-query")]
    pub name: Option<String>,
}

/// `cellgov dev fixture-gen`
#[derive(Debug, clap::Args)]
pub(crate) struct FixtureGenArgs {
    /// The title manifest the fixture is generated for.
    #[arg(long, value_name = "PATH")]
    pub manifest: PathBuf,
    /// CellGov's observation JSON.
    #[arg(long, value_name = "PATH")]
    pub cellgov: String,
    /// The other runner's observation JSON.
    #[arg(long, value_name = "PATH")]
    pub rpcs3: String,
    /// Fixture tree the cell's directory is created under.
    #[arg(long, value_name = "DIR")]
    pub fixtures_dir: Option<PathBuf>,
    /// Write the fixture even when the two observations disagree.
    #[arg(long)]
    pub allow_divergence: bool,
    #[command(flatten)]
    pub selection: BootSelection,
}

/// `cellgov dev titles-gen`
#[derive(Debug, clap::Args)]
pub(crate) struct TitlesGenArgs {
    /// Title registry directory.
    #[arg(long, value_name = "DIR")]
    pub registry: Option<String>,
    /// Cross-runner fixture directory.
    #[arg(long, value_name = "DIR")]
    pub fixtures_dir: Option<String>,
    /// Directory the generated documents are written under.
    #[arg(long, value_name = "DIR")]
    pub output_dir: Option<String>,
}

/// `cellgov dev cli-gen`
#[derive(Debug, clap::Args)]
pub(crate) struct CliGenArgs {
    /// Document to write.
    #[arg(long, value_name = "PATH")]
    pub output: Option<PathBuf>,
}

/// Where each shell reads a completion script from, and the outcome
/// `dev completions` has beyond the shared 0-5 contract.
const COMPLETIONS_INSTALL_NOTE: &str = "\
The script goes to stdout; redirect it to where the shell reads it:

  bash  cellgov dev completions bash > ~/.local/share/bash-completion/completions/cellgov
  zsh   cellgov dev completions zsh > \"${fpath[1]}/_cellgov\"
  pwsh  cellgov dev completions pwsh >> $PROFILE

Exit codes particular to this command:
  141  stdout was closed by a downstream reader";

/// `cellgov dev completions`
#[derive(Debug, clap::Args)]
#[command(after_help = COMPLETIONS_INSTALL_NOTE)]
pub(crate) struct CompletionsArgs {
    /// Shell the script is written for.
    #[arg(value_name = "SHELL", value_enum)]
    pub shell: CompletionShell,
}

/// The shells `dev completions` writes a script for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum CompletionShell {
    Bash,
    Zsh,
    Pwsh,
}

/// Which record `gen-manifest` generates from.
const GEN_MANIFEST_SCOPE: &str = "Notes:
  A title's base record generates that title's manifest; a title-update
  record is refused by name. The stub's system_ver is read from the
  installed tree's PARAM.SFO. That tree, and the record directory
  --title-id and --firmware default to, sit under the store root
  --vfs-root names, so the tree must be present there. A firmware
  record generates the manifest for the system software the firmware
  ships. That manifest names no firmware version: the store holds the
  version, and the manifest resolves against whichever firmware --fw
  selects.";

/// `cellgov dev gen-manifest`
#[derive(Debug, clap::Args)]
#[command(after_help = GEN_MANIFEST_SCOPE)]
#[command(group = clap::ArgGroup::new("gen-manifest-record")
    .required(true)
    .args(["record", "title_id", "firmware"]))]
pub(crate) struct GenManifestArgs {
    /// An install record to read directly.
    #[arg(long, value_name = "PATH", conflicts_with = "installs")]
    pub record: Option<PathBuf>,
    /// A title id whose base record is looked up under `--installs`.
    #[arg(long, value_name = "ID")]
    pub title_id: Option<String>,
    /// A firmware version whose record is looked up under `--installs`.
    #[arg(long, value_name = "VERSION")]
    pub firmware: Option<String>,
    /// Install-records directory `--title-id` and `--firmware` are
    /// resolved under.
    #[arg(long, value_name = "DIR")]
    pub installs: Option<PathBuf>,
    /// Registry directory the stub is written into.
    #[arg(long, value_name = "DIR")]
    pub registry: Option<PathBuf>,
    /// Overwrite an existing manifest.
    #[arg(long)]
    pub force: bool,
}

/// Which titles `cellgov dev record-anchors` re-measures.
#[derive(Debug, clap::Args)]
#[group(required = true, multiple = false)]
pub(crate) struct AnchorScope {
    /// Re-measure every title in the registry.
    #[arg(long)]
    pub all: bool,
    /// Re-measure one title by short name.
    #[arg(long, value_name = "NAME")]
    pub title: Option<String>,
}

/// `cellgov dev record-anchors`
#[derive(Debug, clap::Args)]
pub(crate) struct RecordAnchorsArgs {
    #[command(flatten)]
    pub scope: AnchorScope,
    /// Record only the declared cells at this firmware version.
    #[arg(long, value_name = "VERSION")]
    pub fw: Option<String>,
    /// Record only the declared cells at this game version.
    #[arg(long, value_name = "base|VERSION")]
    pub game_ver: Option<String>,
    /// Registry directory; must be the one the measurement reads.
    #[arg(long, value_name = "DIR")]
    pub registry: Option<PathBuf>,
}
