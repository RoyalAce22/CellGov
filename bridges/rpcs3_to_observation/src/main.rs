//! RPCS3 dump -> `Observation` adapter.
//!
//! Converts the raw binary dump produced by the RPCS3 patch
//! (`bridges/rpcs3-patch/`) into a normalized `Observation` JSON that
//! the CellGov `compare-observations` subcommand can consume.
//!
//! Usage:
//!
//! ```text
//! rpcs3_to_observation --dump <path> --manifest <path> --outcome <kind> \
//!     [--steps <n>] --output <path>
//! ```
//!
//! Where `<kind>` is one of `completed|stalled|timeout|fault`.
//!
//! The manifest is a TOML file matching the one consumed by the
//! RPCS3 patch and by CellGov's own observation producer; see
//! [`Manifest`] for the schema.
//!
//! The adapter performs a single read of the dump, slices it into
//! regions in manifest order, and writes the resulting
//! `Observation` as pretty JSON.

use cellgov_compare::observation::{
    NamedMemoryRegion, Observation, ObservationMetadata, ObservedOutcome,
};
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

/// TOML region-manifest schema.
///
/// Describes the regions the dump contains, in declaration order.
/// The RPCS3 patch writes regions to the dump in the same order, so
/// the adapter reads them off the dump file by walking the manifest
/// and advancing a byte cursor.
#[derive(Debug, Deserialize)]
struct Manifest {
    regions: Vec<ManifestRegion>,
}

/// One named guest-memory region entry in the manifest.
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

fn parse_args(argv: Vec<String>) -> Result<Args, String> {
    let mut dump: Option<PathBuf> = None;
    let mut manifest: Option<PathBuf> = None;
    let mut outcome: Option<ObservedOutcome> = None;
    let mut steps: Option<usize> = None;
    let mut output: Option<PathBuf> = None;

    let mut it = argv.into_iter().skip(1);
    while let Some(flag) = it.next() {
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
            other => return Err(format!("unknown flag: {other}")),
        }
    }

    Ok(Args {
        dump: dump.ok_or("--dump required")?,
        manifest: manifest.ok_or("--manifest required")?,
        outcome: outcome.ok_or("--outcome required")?,
        steps,
        output: output.ok_or("--output required")?,
    })
}

/// Convert a dump + manifest into an `Observation`.
///
/// Pure function so it can be tested without file I/O. Returns an
/// error if the dump is smaller than the sum of manifest region
/// sizes, which means the RPCS3 side wrote fewer bytes than the
/// manifest promised (usually a mismatched manifest).
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

fn run(args: Args) -> Result<(), String> {
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
    match parse_args(argv).and_then(run) {
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
    fn flow_checkpoint_manifest_parses_and_fits_guest_memory() {
        // The checked-in flOw manifest must parse and its
        // regions must fit inside CellGov's 1 GB guest memory. Run-game
        // uses 0x4000_0000 bytes (see apps/cellgov_cli/src/game.rs).
        let root = env!("CARGO_MANIFEST_DIR");
        let path = std::path::Path::new(root)
            .join("..")
            .join("..")
            .join("tests")
            .join("fixtures")
            .join("flow_checkpoint.toml");
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
}
