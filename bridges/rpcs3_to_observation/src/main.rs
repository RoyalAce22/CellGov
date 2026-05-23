//! Adapter from RPCS3 dump + manifest into `cellgov_compare::Observation` JSON.
//!
//! ```text
//! rpcs3_to_observation --dump <path> --manifest <path> --outcome <kind> \
//!     --config-hash <hex> [--steps <n>] --output <path>
//! rpcs3_to_observation --print-expected-config-hash
//! ```
//!
//! `<kind>` is one of `completed|stalled|timeout|fault`. `--config-hash` is the
//! 16-char hex FNV-1a of the canonical oracle-mode YAML; a mismatch is rejected.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "CLI binary: stdout/stderr are the user-facing output channel"
)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use cellgov_compare::observation::{
    NamedMemoryRegion, Observation, ObservationMetadata, ObservedOutcome,
};
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

/// Region list in dump order; the RPCS3 patch writes regions in the same order.
#[derive(Debug, Deserialize)]
struct Manifest {
    regions: Vec<ManifestRegion>,
}

#[derive(Debug, Deserialize)]
struct ManifestRegion {
    name: String,
    #[serde(deserialize_with = "de_hex_u64")]
    addr: u64,
    #[serde(deserialize_with = "de_hex_u64")]
    size: u64,
}

fn de_hex_u64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    let s = String::deserialize(d)?;
    let trimmed = s.strip_prefix("0x").unwrap_or(&s);
    u64::from_str_radix(trimmed, 16).map_err(serde::de::Error::custom)
}

struct Args {
    dump: PathBuf,
    manifest: PathBuf,
    outcome: ObservedOutcome,
    steps: Option<usize>,
    output: PathBuf,
    config_hash: u64,
}

/// Canonical RPCS3 oracle-mode config; dumps produced under any other settings
/// are not cross-runner comparable and are rejected at conversion time.
const ORACLE_MODE_CONFIG_YAML: &str = include_str!("../../rpcs3-patch/oracle_mode_config.yml");

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

fn expected_config_hash() -> u64 {
    fnv1a_64(ORACLE_MODE_CONFIG_YAML.as_bytes())
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
    /// rpcs3 oracle-mode config hash disagrees with the patch source.
    #[error(
        "rpcs3 oracle-mode config mismatch: supplied 0x{supplied:016x}, expected 0x{expected:016x}. \
         The dump was produced under RPCS3 settings that differ from \
         bridges/rpcs3-patch/oracle_mode_config.yml. Cross-runner \
         observations from different settings are not comparable; \
         re-run RPCS3 with the canonical oracle-mode settings."
    )]
    ConfigHashMismatch { supplied: u64, expected: u64 },
    /// Reading the manifest file failed.
    #[error("read manifest {}: {source}", path.display())]
    ManifestRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Parsing the manifest TOML failed.
    #[error("parse manifest: {0}")]
    ManifestParse(#[source] toml::de::Error),
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

fn parse_args(argv: Vec<String>) -> Result<ParsedArgs, Rpcs3BridgeError> {
    let mut dump: Option<PathBuf> = None;
    let mut manifest: Option<PathBuf> = None;
    let mut outcome: Option<ObservedOutcome> = None;
    let mut steps: Option<usize> = None;
    let mut output: Option<PathBuf> = None;
    let mut config_hash: Option<u64> = None;

    let mut it = argv.into_iter().skip(1);
    while let Some(flag) = it.next() {
        if flag == "--print-expected-config-hash" {
            return Ok(ParsedArgs::PrintExpectedConfigHash);
        }
        let val = it
            .next()
            .ok_or_else(|| Rpcs3BridgeError::FlagMissingValue { flag: flag.clone() })?;
        match flag.as_str() {
            "--dump" => dump = Some(PathBuf::from(val)),
            "--manifest" => manifest = Some(PathBuf::from(val)),
            "--outcome" => outcome = Some(parse_outcome(&val)?),
            "--steps" => {
                steps = Some(val.parse().map_err(Rpcs3BridgeError::InvalidSteps)?);
            }
            "--output" => output = Some(PathBuf::from(val)),
            "--config-hash" => config_hash = Some(parse_hex_u64(&val)?),
            other => return Err(Rpcs3BridgeError::UnknownFlag(other.to_string())),
        }
    }

    Ok(ParsedArgs::Convert(Args {
        dump: dump.ok_or(Rpcs3BridgeError::RequiredFlagMissing { flag: "--dump" })?,
        manifest: manifest.ok_or(Rpcs3BridgeError::RequiredFlagMissing { flag: "--manifest" })?,
        outcome: outcome.ok_or(Rpcs3BridgeError::RequiredFlagMissing { flag: "--outcome" })?,
        steps,
        output: output.ok_or(Rpcs3BridgeError::RequiredFlagMissing { flag: "--output" })?,
        config_hash: config_hash.ok_or(Rpcs3BridgeError::RequiredFlagMissing {
            flag: "--config-hash",
        })?,
    }))
}

/// Slice `dump` into regions by walking the manifest and advancing a byte cursor.
///
/// # Errors
///
/// Returns `Err` when the dump is shorter than the sum of manifest region sizes.
fn build_observation(
    dump: &[u8],
    manifest: &Manifest,
    outcome: ObservedOutcome,
    steps: Option<usize>,
) -> Result<Observation, Rpcs3BridgeError> {
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

    Ok(Observation {
        outcome,
        memory_regions: regions,
        events: vec![],
        state_hashes: None,
        metadata: ObservationMetadata {
            runner: "rpcs3".into(),
            steps,
        },
        // The bridge consumes RPCS3's magic-tagged region payload, not
        // the surrounding TTY stream; left empty.
        tty_log: Vec::new(),
    })
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

    let manifest_text =
        fs::read_to_string(&args.manifest).map_err(|source| Rpcs3BridgeError::ManifestRead {
            path: args.manifest.clone(),
            source,
        })?;
    let manifest: Manifest =
        toml::from_str(&manifest_text).map_err(Rpcs3BridgeError::ManifestParse)?;

    let dump = fs::read(&args.dump).map_err(|source| Rpcs3BridgeError::DumpRead {
        path: args.dump.clone(),
        source,
    })?;

    let obs = build_observation(&dump, &manifest, args.outcome, args.steps)?;

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
mod tests {
    use super::*;
    use cellgov_compare::observation::ObservedOutcome;

    fn manifest_fixture() -> Manifest {
        Manifest {
            regions: vec![
                ManifestRegion {
                    name: "first".into(),
                    addr: 0x10000,
                    size: 4,
                },
                ManifestRegion {
                    name: "second".into(),
                    addr: 0x20000,
                    size: 8,
                },
            ],
        }
    }

    #[test]
    fn dump_slices_contiguously_in_manifest_order() {
        let dump: Vec<u8> = (0..12).collect();
        let manifest = manifest_fixture();
        let obs = build_observation(&dump, &manifest, ObservedOutcome::Completed, Some(42))
            .expect("builds");
        assert_eq!(obs.memory_regions.len(), 2);
        assert_eq!(obs.memory_regions[0].name, "first");
        assert_eq!(obs.memory_regions[0].addr, 0x10000);
        assert_eq!(obs.memory_regions[0].data, vec![0, 1, 2, 3]);
        assert_eq!(obs.memory_regions[1].name, "second");
        assert_eq!(obs.memory_regions[1].addr, 0x20000);
        assert_eq!(obs.memory_regions[1].data, vec![4, 5, 6, 7, 8, 9, 10, 11]);
        assert_eq!(obs.metadata.runner, "rpcs3");
        assert_eq!(obs.metadata.steps, Some(42));
        assert!(obs.state_hashes.is_none());
    }

    #[test]
    fn truncated_dump_is_rejected_with_named_region() {
        let dump: Vec<u8> = vec![0; 10];
        let manifest = manifest_fixture();
        let err = build_observation(&dump, &manifest, ObservedOutcome::Completed, None)
            .expect_err("truncated");
        match err {
            Rpcs3BridgeError::DumpTruncated { region, .. } => assert_eq!(region, "second"),
            other => panic!("expected DumpTruncated(second), got {other:?}"),
        }
    }

    #[test]
    fn observation_roundtrips_through_json() {
        let dump: Vec<u8> = (0..12).collect();
        let manifest = manifest_fixture();
        let obs = build_observation(&dump, &manifest, ObservedOutcome::Fault, None).unwrap();
        let json = serde_json::to_string(&obs).unwrap();
        let back: Observation = serde_json::from_str(&json).unwrap();
        assert_eq!(obs, back);
    }

    #[test]
    fn outcome_parser_accepts_all_kinds() {
        assert_eq!(
            parse_outcome("completed").unwrap(),
            ObservedOutcome::Completed
        );
        assert_eq!(parse_outcome("stalled").unwrap(), ObservedOutcome::Stalled);
        assert_eq!(parse_outcome("timeout").unwrap(), ObservedOutcome::Timeout);
        assert_eq!(parse_outcome("fault").unwrap(), ObservedOutcome::Fault);
        assert_eq!(
            parse_outcome("process_exit").unwrap(),
            ObservedOutcome::ProcessExit
        );
        assert_eq!(
            parse_outcome("process-exit").unwrap(),
            ObservedOutcome::ProcessExit
        );
        assert!(parse_outcome("bogus").is_err());
    }

    #[test]
    fn checkpoint_manifest_parses_and_fits_guest_memory() {
        let root = env!("CARGO_MANIFEST_DIR");
        let path = std::path::Path::new(root)
            .join("..")
            .join("..")
            .join("tests")
            .join("fixtures")
            .join("NPUA80001")
            .join("checkpoint.toml");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let m: Manifest = toml::from_str(&text).expect("manifest parses");
        assert!(!m.regions.is_empty(), "manifest has at least one region");

        const GUEST_MEM: u64 = 0x4000_0000;
        for r in &m.regions {
            let end = r
                .addr
                .checked_add(r.size)
                .unwrap_or_else(|| panic!("region {} addr+size overflows", r.name));
            assert!(
                end <= GUEST_MEM,
                "region {} ({}..{}) exceeds 1GB guest memory",
                r.name,
                r.addr,
                end
            );
        }
    }

    #[test]
    fn manifest_parses_hex_addresses() {
        let toml = r#"
            [[regions]]
            name = "code"
            addr = "0x10000"
            size = "0x800000"
        "#;
        let m: Manifest = toml::from_str(toml).unwrap();
        assert_eq!(m.regions[0].addr, 0x10000);
        assert_eq!(m.regions[0].size, 0x800000);
    }

    #[test]
    fn fnv1a_64_matches_known_vector() {
        assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a_64(b"abc"), 0xe71f_a219_0541_574b);
    }

    #[test]
    fn expected_config_hash_is_stable_and_nonzero() {
        let h = expected_config_hash();
        assert_ne!(h, 0);
        assert_eq!(h, expected_config_hash());
    }

    #[test]
    fn check_config_hash_accepts_expected() {
        assert!(check_config_hash(expected_config_hash()).is_ok());
    }

    #[test]
    fn check_config_hash_rejects_with_diagnostic_naming_both_sides() {
        let err = check_config_hash(0xdead_beef_dead_beef).expect_err("mismatch");
        let rendered = err.to_string();
        assert!(
            rendered.contains("oracle-mode config mismatch"),
            "names the contract: {rendered}"
        );
        assert!(
            rendered.contains("0xdeadbeefdeadbeef"),
            "echoes supplied hash: {rendered}"
        );
        let expected = format!("0x{:016x}", expected_config_hash());
        assert!(
            rendered.contains(&expected),
            "names expected hash {expected}: {rendered}"
        );
    }

    #[test]
    fn parse_hex_u64_accepts_both_prefixed_and_bare() {
        assert_eq!(parse_hex_u64("0xabc").unwrap(), 0xabc);
        assert_eq!(parse_hex_u64("abc").unwrap(), 0xabc);
        assert!(parse_hex_u64("xyz").is_err());
    }

    #[test]
    fn oracle_mode_yaml_contains_four_required_settings() {
        let y = ORACLE_MODE_CONFIG_YAML;
        assert!(y.contains("Renderer: \"Null\""));
        assert!(y.contains("PPU Decoder: Recompiler (LLVM)"));
        assert!(y.contains("SPU Decoder: Recompiler (LLVM)"));
    }
}
