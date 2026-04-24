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

fn parse_hex_u64(s: &str) -> Result<u64, String> {
    let trimmed = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(trimmed, 16).map_err(|e| format!("invalid hex '{s}': {e}"))
}

fn parse_outcome(s: &str) -> Result<ObservedOutcome, String> {
    match s {
        "completed" => Ok(ObservedOutcome::Completed),
        "stalled" => Ok(ObservedOutcome::Stalled),
        "timeout" => Ok(ObservedOutcome::Timeout),
        "fault" => Ok(ObservedOutcome::Fault),
        other => Err(format!("unknown outcome: {other}")),
    }
}

enum ParsedArgs {
    Convert(Args),
    PrintExpectedConfigHash,
}

fn parse_args(argv: Vec<String>) -> Result<ParsedArgs, String> {
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
            .ok_or_else(|| format!("flag {flag} requires a value"))?;
        match flag.as_str() {
            "--dump" => dump = Some(PathBuf::from(val)),
            "--manifest" => manifest = Some(PathBuf::from(val)),
            "--outcome" => outcome = Some(parse_outcome(&val)?),
            "--steps" => {
                steps = Some(val.parse().map_err(|e| format!("--steps: {e}"))?);
            }
            "--output" => output = Some(PathBuf::from(val)),
            "--config-hash" => config_hash = Some(parse_hex_u64(&val)?),
            other => return Err(format!("unknown flag: {other}")),
        }
    }

    Ok(ParsedArgs::Convert(Args {
        dump: dump.ok_or("--dump required")?,
        manifest: manifest.ok_or("--manifest required")?,
        outcome: outcome.ok_or("--outcome required")?,
        steps,
        output: output.ok_or("--output required")?,
        config_hash: config_hash.ok_or("--config-hash required")?,
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
) -> Result<Observation, String> {
    let mut cursor: usize = 0;
    let mut regions = Vec::with_capacity(manifest.regions.len());
    for r in &manifest.regions {
        let size = r.size as usize;
        let end = cursor
            .checked_add(size)
            .ok_or_else(|| format!("region {} size overflow", r.name))?;
        if end > dump.len() {
            return Err(format!(
                "dump truncated: region {} needs bytes [{cursor}..{end}] but dump has {}",
                r.name,
                dump.len()
            ));
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
    })
}

fn check_config_hash(supplied: u64) -> Result<(), String> {
    let expected = expected_config_hash();
    if supplied != expected {
        return Err(format!(
            "rpcs3 oracle-mode config mismatch: supplied 0x{supplied:016x}, expected 0x{expected:016x}. \
             The dump was produced under RPCS3 settings that differ from \
             bridges/rpcs3-patch/oracle_mode_config.yml. Cross-runner \
             observations from different settings are not comparable; \
             re-run RPCS3 with the canonical oracle-mode settings."
        ));
    }
    Ok(())
}

fn run(args: Args) -> Result<(), String> {
    check_config_hash(args.config_hash)?;

    let manifest_text = fs::read_to_string(&args.manifest)
        .map_err(|e| format!("read manifest {}: {e}", args.manifest.display()))?;
    let manifest: Manifest =
        toml::from_str(&manifest_text).map_err(|e| format!("parse manifest: {e}"))?;

    let dump =
        fs::read(&args.dump).map_err(|e| format!("read dump {}: {e}", args.dump.display()))?;

    let obs = build_observation(&dump, &manifest, args.outcome, args.steps)?;

    let json = serde_json::to_string_pretty(&obs).map_err(|e| format!("serialize: {e}"))?;
    fs::write(&args.output, json).map_err(|e| format!("write {}: {e}", args.output.display()))?;
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
        assert!(
            err.contains("second"),
            "error names the truncated region: {err}"
        );
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
    fn outcome_parser_accepts_four_kinds() {
        assert_eq!(parse_outcome("completed"), Ok(ObservedOutcome::Completed));
        assert_eq!(parse_outcome("stalled"), Ok(ObservedOutcome::Stalled));
        assert_eq!(parse_outcome("timeout"), Ok(ObservedOutcome::Timeout));
        assert_eq!(parse_outcome("fault"), Ok(ObservedOutcome::Fault));
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
            .join("NPUA80001_checkpoint.toml");
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
        assert!(
            err.contains("oracle-mode config mismatch"),
            "names the contract: {err}"
        );
        assert!(
            err.contains("0xdeadbeefdeadbeef"),
            "echoes supplied hash: {err}"
        );
        let expected = format!("0x{:016x}", expected_config_hash());
        assert!(
            err.contains(&expected),
            "names expected hash {expected}: {err}"
        );
    }

    #[test]
    fn parse_hex_u64_accepts_both_prefixed_and_bare() {
        assert_eq!(parse_hex_u64("0xabc"), Ok(0xabc));
        assert_eq!(parse_hex_u64("abc"), Ok(0xabc));
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
