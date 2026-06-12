//! Observation assembly from raw RPCS3 dumps: manifest-ordered slicing, truncation rejection, and outcome parsing.

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
    let obs =
        build_observation(&dump, &manifest, ObservedOutcome::Completed, Some(42)).expect("builds");
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
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
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
