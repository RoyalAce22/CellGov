//! Adapter from RPCS3 dump + manifest into `cellgov_compare::Observation` JSON.
//!
//! ```text
//! rpcs3_to_observation (--dump <path> | --tty <path>) --manifest <path> \
//!     --outcome <kind> \
//!     --decoder <interpreter|llvm> --config-hash <hex> [--steps <n>] \
//!     [--rpcs3-dir <path>] --output <path>
//! rpcs3_to_observation --print-expected-config-hash
//! ```
//!
//! `<kind>` is one of `completed|stalled|timeout|fault`. `--config-hash` is
//! the 16-char hex FNV-1a of the hashed block in the canonical config YAML.
//! `--decoder` records which decoder ran; an `--output` whose name ends
//! `_<decoder>.json` must name that same decoder. `--rpcs3-dir` names the
//! runner installation the capture came from; the adapter stamps the
//! observation with the firmware version it finds there. A synthetic
//! scenario runs against no firmware, so the flag is optional;
//! `cellgov dev fixture-gen` refuses a title capture that omits it.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "CLI binary: stdout/stderr are the user-facing output channel"
)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use cellgov_compare::checkpoint_manifest::{self, CheckpointManifest, CheckpointManifestError};
use cellgov_compare::observation::{
    NamedMemoryRegion, Observation, ObservationMetadata, ObservedOutcome,
};
use cellgov_compare::runner_rpcs3::{parse_tty_log, TtyRegion};
use runner_firmware::RunnerFirmwareError;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod runner_firmware;

/// Where the region bytes come from. RPCS3 produces one or the
/// other: a binary memory dump from the checkpoint hook, or its
/// TTY log carrying a `CGOV`-framed payload.
enum Capture {
    Dump(PathBuf),
    Tty(PathBuf),
}

struct Args {
    capture: Capture,
    manifest: PathBuf,
    outcome: ObservedOutcome,
    steps: Option<usize>,
    output: PathBuf,
    config_hash: u64,
    decoder: Decoder,
    rpcs3_dir: Option<PathBuf>,
}

/// Canonical RPCS3 reference-mode config. Dumps produced under other
/// settings compare against nothing meaningful and are rejected at
/// conversion time.
///
/// Kept beside this crate: `bridges/rpcs3-patch/` is GPL-2.0-only and
/// this binary is Apache-2.0 / MIT.
const REFERENCE_MODE_CONFIG_YAML: &str = include_str!("../oracle_mode_config.yml");

const HASHED_BEGIN: &str = "# --- BEGIN HASHED ---";
const HASHED_END: &str = "# --- END HASHED ---";

/// Decoder a capture ran under. Recorded per capture and kept out of
/// the config hash, so both variants of a scenario stay reproducible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decoder {
    Interpreter,
    Llvm,
}

impl Decoder {
    /// Every decoder token, for checking an output name against the ones
    /// this capture did not run under.
    const ALL: [Decoder; 2] = [Decoder::Interpreter, Decoder::Llvm];

    /// The `metadata.runner` tail, and the token an output filename may
    /// carry as `_<name>.json`.
    fn name(self) -> &'static str {
        match self {
            Self::Interpreter => "interpreter",
            Self::Llvm => "llvm",
        }
    }
}

fn parse_decoder(s: &str) -> Result<Decoder, Rpcs3BridgeError> {
    match s {
        "interpreter" => Ok(Decoder::Interpreter),
        "llvm" => Ok(Decoder::Llvm),
        other => Err(Rpcs3BridgeError::UnknownDecoder(other.to_string())),
    }
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// The lines between the two markers: the settings every capture must
/// share. Everything outside varies per capture or is commentary.
///
/// # Panics
///
/// If either marker is absent or they appear out of order. The file is
/// `include_str!`d from this repo, so a missing marker is a build-time
/// editing mistake, and hashing the whole file instead would silently
/// restore the behaviour this split exists to remove.
fn hashed_config_section(yaml: &str) -> &str {
    let after_begin = yaml
        .split_once(HASHED_BEGIN)
        .expect("invariant: oracle_mode_config.yml carries the BEGIN HASHED marker")
        .1;
    after_begin
        .split_once(HASHED_END)
        .expect("invariant: oracle_mode_config.yml carries the END HASHED marker after BEGIN")
        .0
}

fn expected_config_hash() -> u64 {
    fnv1a_64(hashed_config_section(REFERENCE_MODE_CONFIG_YAML).as_bytes())
}

/// Why the rpcs3 to-observation bridge failed.
#[derive(Debug, thiserror::Error)]
enum Rpcs3BridgeError {
    /// Hex parse failed.
    #[error("invalid hex '{raw}': {source}")]
    InvalidHex {
        raw: String,
        #[source]
        source: std::num::ParseIntError,
    },
    /// Outcome token unrecognized.
    #[error("unknown outcome: {0}")]
    UnknownOutcome(String),
    /// Both capture sources named at once.
    #[error("--dump and --tty name two different captures; pass one")]
    CaptureSourceAmbiguous,
    /// The same flag was given more than once.
    #[error(
        "{flag} given more than once; the later value would silently win, \
         and which capture the observation describes would depend on \
         argument order"
    )]
    DuplicateFlag { flag: String },
    /// The TTY log could not be parsed into the declared regions.
    #[error("parse tty log: {0}")]
    TtyParse(#[source] cellgov_compare::runner_rpcs3::Rpcs3Error),
    /// Decoder token unrecognized.
    #[error("unknown decoder: {0} (accepted: interpreter, llvm)")]
    UnknownDecoder(String),
    /// The output filename names a decoder other than the one passed.
    #[error(
        "--decoder {decoder} but output {} is named for the {named} \
         decoder. Writing it would file one decoder's answer under the \
         other's name.",
        output.display()
    )]
    DecoderFilenameMismatch {
        decoder: &'static str,
        named: &'static str,
        output: PathBuf,
    },
    /// Two manifest regions share a name.
    #[error(
        "manifest declares region {region} twice; observations are matched \
         region-by-name, so the second copy would never be compared"
    )]
    DuplicateRegionName { region: String },
    /// The manifest declares nothing to extract.
    #[error(
        "manifest declares no regions; the observation would carry no \
         guest-visible state and compare as a match against anything"
    )]
    ManifestHasNoRegions,
    /// A region names an address space no RPCS3 capture can hold.
    #[error(
        "region {region} names address space {space}, but RPCS3 emulates one \
         guest process in one flat address space, so no RPCS3 capture holds a \
         child space; observe it on the CellGov side, or move it to space 0"
    )]
    ChildSpaceRegion { region: String, space: u32 },
    /// Dump longer than the regions the manifest declares.
    #[error(
        "dump has {dump_len} bytes but the manifest declares {declared}; the \
         dump is exactly the declared regions concatenated, so a surplus \
         means this manifest does not describe this dump"
    )]
    DumpLongerThanManifest { declared: usize, dump_len: usize },
    /// CLI flag with no following value.
    #[error("flag {flag} requires a value")]
    FlagMissingValue { flag: String },
    /// `--steps` value did not parse as usize.
    #[error("--steps: {0}")]
    InvalidSteps(#[source] std::num::ParseIntError),
    /// Unknown CLI flag.
    #[error("unknown flag: {0}")]
    UnknownFlag(String),
    /// A required CLI flag was missing.
    #[error("{flag} required")]
    RequiredFlagMissing { flag: &'static str },
    /// The runner installation named no readable firmware version.
    #[error("--rpcs3-dir: {0}")]
    RunnerFirmware(#[from] RunnerFirmwareError),
    /// `region.size` overflowed usize while accumulating cursor.
    #[error("region {region} size overflow")]
    RegionSizeOverflow { region: String },
    /// Dump file shorter than manifest's declared regions.
    #[error(
        "dump truncated: region {region} needs bytes [{cursor}..{end}] but dump has {dump_len}"
    )]
    DumpTruncated {
        region: String,
        cursor: usize,
        end: usize,
        dump_len: usize,
    },
    /// rpcs3 reference-mode config hash disagrees with the patch source.
    #[error(
        "rpcs3 reference-mode config mismatch: supplied 0x{supplied:016x}, expected 0x{expected:016x}. \
         The dump was produced under RPCS3 settings that differ from the \
         hashed block of bridges/rpcs3_to_observation/oracle_mode_config.yml. \
         Captures made under different settings compare against nothing \
         meaningful; re-run RPCS3 with those settings. The decoder pair \
         sits outside the hash -- pass it as --decoder instead."
    )]
    ConfigHashMismatch { supplied: u64, expected: u64 },
    /// The manifest file could not be read or is not a manifest.
    #[error("manifest: {0}")]
    Manifest(#[from] CheckpointManifestError),
    /// Reading the dump file failed.
    #[error("read dump {}: {source}", path.display())]
    DumpRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Serializing the observation to JSON failed.
    #[error("serialize: {0}")]
    Serialize(#[source] serde_json::Error),
    /// Writing the output file failed.
    #[error("write {}: {source}", path.display())]
    OutputWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

fn parse_hex_u64(s: &str) -> Result<u64, Rpcs3BridgeError> {
    let trimmed = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(trimmed, 16).map_err(|source| Rpcs3BridgeError::InvalidHex {
        raw: s.to_string(),
        source,
    })
}

fn parse_outcome(s: &str) -> Result<ObservedOutcome, Rpcs3BridgeError> {
    match s {
        "completed" => Ok(ObservedOutcome::Completed),
        "process_exit" | "process-exit" => Ok(ObservedOutcome::ProcessExit),
        "stalled" => Ok(ObservedOutcome::Stalled),
        "timeout" => Ok(ObservedOutcome::Timeout),
        "fault" => Ok(ObservedOutcome::Fault),
        other => Err(Rpcs3BridgeError::UnknownOutcome(other.to_string())),
    }
}

enum ParsedArgs {
    Convert(Args),
    PrintExpectedConfigHash,
}

/// Fill a not-yet-set slot, refusing a flag that already has a value.
///
/// # Errors
///
/// Returns `Err` when `slot` is already `Some`.
fn set_once<T>(slot: &mut Option<T>, flag: &str, value: T) -> Result<(), Rpcs3BridgeError> {
    if slot.is_some() {
        return Err(Rpcs3BridgeError::DuplicateFlag {
            flag: flag.to_string(),
        });
    }
    *slot = Some(value);
    Ok(())
}

fn parse_args(argv: Vec<String>) -> Result<ParsedArgs, Rpcs3BridgeError> {
    let mut dump: Option<PathBuf> = None;
    let mut tty: Option<PathBuf> = None;
    let mut manifest: Option<PathBuf> = None;
    let mut outcome: Option<ObservedOutcome> = None;
    let mut steps: Option<usize> = None;
    let mut output: Option<PathBuf> = None;
    let mut config_hash: Option<u64> = None;
    let mut decoder: Option<Decoder> = None;
    let mut rpcs3_dir: Option<PathBuf> = None;

    let mut it = argv.into_iter().skip(1);
    while let Some(flag) = it.next() {
        if flag == "--print-expected-config-hash" {
            return Ok(ParsedArgs::PrintExpectedConfigHash);
        }
        let val = it
            .next()
            .ok_or_else(|| Rpcs3BridgeError::FlagMissingValue { flag: flag.clone() })?;
        match flag.as_str() {
            "--dump" => set_once(&mut dump, &flag, PathBuf::from(val))?,
            "--tty" => set_once(&mut tty, &flag, PathBuf::from(val))?,
            "--manifest" => set_once(&mut manifest, &flag, PathBuf::from(val))?,
            "--outcome" => set_once(&mut outcome, &flag, parse_outcome(&val)?)?,
            "--steps" => set_once(
                &mut steps,
                &flag,
                val.parse().map_err(Rpcs3BridgeError::InvalidSteps)?,
            )?,
            "--output" => set_once(&mut output, &flag, PathBuf::from(val))?,
            "--config-hash" => set_once(&mut config_hash, &flag, parse_hex_u64(&val)?)?,
            "--decoder" => set_once(&mut decoder, &flag, parse_decoder(&val)?)?,
            "--rpcs3-dir" => set_once(&mut rpcs3_dir, &flag, PathBuf::from(val))?,
            other => return Err(Rpcs3BridgeError::UnknownFlag(other.to_string())),
        }
    }

    Ok(ParsedArgs::Convert(Args {
        capture: match (dump, tty) {
            (Some(d), None) => Capture::Dump(d),
            (None, Some(t)) => Capture::Tty(t),
            (Some(_), Some(_)) => return Err(Rpcs3BridgeError::CaptureSourceAmbiguous),
            (None, None) => {
                return Err(Rpcs3BridgeError::RequiredFlagMissing {
                    flag: "--dump or --tty",
                })
            }
        },
        manifest: manifest.ok_or(Rpcs3BridgeError::RequiredFlagMissing { flag: "--manifest" })?,
        outcome: outcome.ok_or(Rpcs3BridgeError::RequiredFlagMissing { flag: "--outcome" })?,
        steps,
        output: output.ok_or(Rpcs3BridgeError::RequiredFlagMissing { flag: "--output" })?,
        config_hash: config_hash.ok_or(Rpcs3BridgeError::RequiredFlagMissing {
            flag: "--config-hash",
        })?,
        decoder: decoder.ok_or(Rpcs3BridgeError::RequiredFlagMissing { flag: "--decoder" })?,
        rpcs3_dir,
    }))
}

/// Reject a manifest that cannot produce a comparable observation.
///
/// # Errors
///
/// Returns `Err` on an empty region list, on two regions sharing a
/// name, or on a region outside space 0.
fn check_manifest(manifest: &CheckpointManifest) -> Result<(), Rpcs3BridgeError> {
    if manifest.regions.is_empty() {
        return Err(Rpcs3BridgeError::ManifestHasNoRegions);
    }
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for r in &manifest.regions {
        if r.space != 0 {
            return Err(Rpcs3BridgeError::ChildSpaceRegion {
                region: r.name.clone(),
                space: r.space,
            });
        }
        // `find_memory_divergence` in cellgov_compare pairs regions by
        // name and takes the first match, so a repeated name hides the
        // later region from every comparison.
        if !seen.insert(r.name.as_str()) {
            return Err(Rpcs3BridgeError::DuplicateRegionName {
                region: r.name.clone(),
            });
        }
    }
    Ok(())
}

/// Cut the manifest's regions out of a memory dump. Regions sit
/// contiguously in declaration order, matching how the hook wrote them.
///
/// # Errors
///
/// Returns `Err` when the dump does not hold exactly the declared
/// regions: short of them, or longer than their total.
fn slice_dump(
    dump: &[u8],
    manifest: &CheckpointManifest,
) -> Result<Vec<NamedMemoryRegion>, Rpcs3BridgeError> {
    let mut cursor: usize = 0;
    let mut regions = Vec::with_capacity(manifest.regions.len());
    for r in &manifest.regions {
        let size = r.size as usize;
        let end = cursor
            .checked_add(size)
            .ok_or_else(|| Rpcs3BridgeError::RegionSizeOverflow {
                region: r.name.clone(),
            })?;
        if end > dump.len() {
            return Err(Rpcs3BridgeError::DumpTruncated {
                region: r.name.clone(),
                cursor,
                end,
                dump_len: dump.len(),
            });
        }
        regions.push(NamedMemoryRegion {
            name: r.name.clone(),
            addr: r.addr,
            data: dump[cursor..end].to_vec(),
        });
        cursor = end;
    }

    // The dump hook appends each declared region and nothing else, so a
    // dump with bytes left over was produced from a different region
    // list than this manifest names -- see bridges/rpcs3-patch/README.md.
    if cursor != dump.len() {
        return Err(Rpcs3BridgeError::DumpLongerThanManifest {
            declared: cursor,
            dump_len: dump.len(),
        });
    }

    Ok(regions)
}

/// Pack extracted regions into an observation.
fn build_observation(
    memory_regions: Vec<NamedMemoryRegion>,
    outcome: ObservedOutcome,
    steps: Option<usize>,
    decoder: Decoder,
    runner_firmware: Option<String>,
) -> Observation {
    Observation {
        outcome,
        memory_regions,
        events: vec![],
        state_hashes: None,
        metadata: ObservationMetadata {
            runner: format!("rpcs3-{}", decoder.name()),
            steps,
        },
        // The regions above carry the payload; the surrounding TTY
        // stream is left out.
        tty_log: Vec::new(),
        // The runner composes no store entry, so it names no title
        // version and no firmware the store identifies -- only the
        // version its own installation reports.
        identity: cellgov_compare::RunIdentity::default(),
        runner_firmware,
    }
}

/// Refuse an output filename that names a decoder this capture did not
/// run under.
///
/// A name claiming no decoder is accepted: the per-title fixture tree
/// writes a fixed `rpcs3/observation.json` that `cellgov_cli
/// fixture-gen --rpcs3` reads by that name, and `metadata.runner`
/// carries the decoder inside the file either way.
fn check_output_names_decoder(output: &Path, decoder: Decoder) -> Result<(), Rpcs3BridgeError> {
    let name = output.file_name().and_then(|n| n.to_str()).unwrap_or("");
    for other in Decoder::ALL {
        if other != decoder && name.ends_with(&format!("_{}.json", other.name())) {
            return Err(Rpcs3BridgeError::DecoderFilenameMismatch {
                decoder: decoder.name(),
                named: other.name(),
                output: output.to_path_buf(),
            });
        }
    }
    Ok(())
}

fn check_config_hash(supplied: u64) -> Result<(), Rpcs3BridgeError> {
    let expected = expected_config_hash();
    if supplied != expected {
        return Err(Rpcs3BridgeError::ConfigHashMismatch { supplied, expected });
    }
    Ok(())
}

fn run(args: Args) -> Result<(), Rpcs3BridgeError> {
    check_config_hash(args.config_hash)?;
    check_output_names_decoder(&args.output, args.decoder)?;

    let manifest = checkpoint_manifest::load(&args.manifest)?;
    check_manifest(&manifest)?;

    let regions = match &args.capture {
        Capture::Dump(path) => {
            let dump = fs::read(path).map_err(|source| Rpcs3BridgeError::DumpRead {
                path: path.clone(),
                source,
            })?;
            slice_dump(&dump, &manifest)?
        }
        Capture::Tty(path) => {
            let tty_regions: Vec<TtyRegion> = manifest
                .regions
                .iter()
                .map(|r| TtyRegion {
                    name: r.name.clone(),
                    // The manifest's addr is a position inside the
                    // emitted struct, and the observation reports the
                    // same number.
                    offset: r.addr,
                    size: r.size,
                    guest_addr: r.addr,
                })
                .collect();
            parse_tty_log(path, &tty_regions).map_err(Rpcs3BridgeError::TtyParse)?
        }
    };

    // Read now, while the installation is still the one that produced
    // this capture. A later read names whatever firmware the runner
    // holds then.
    let firmware = args
        .rpcs3_dir
        .as_deref()
        .map(runner_firmware::firmware_version)
        .transpose()?;
    let obs = build_observation(regions, args.outcome, args.steps, args.decoder, firmware);

    let json = serde_json::to_string_pretty(&obs).map_err(Rpcs3BridgeError::Serialize)?;
    fs::write(&args.output, json).map_err(|source| Rpcs3BridgeError::OutputWrite {
        path: args.output.clone(),
        source,
    })?;
    Ok(())
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let parsed = match parse_args(argv) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = match parsed {
        ParsedArgs::Convert(args) => run(args),
        ParsedArgs::PrintExpectedConfigHash => {
            println!("0x{:016x}", expected_config_hash());
            Ok(())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
#[path = "tests/main_tests.rs"]
mod tests;
