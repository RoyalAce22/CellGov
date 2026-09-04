//! Observation assembly from raw RPCS3 dumps: manifest-ordered slicing, truncation rejection, and outcome parsing.

use super::*;
use cellgov_compare::checkpoint_manifest::CheckpointRegion;
use cellgov_compare::observation::ObservedOutcome;

fn manifest_fixture() -> CheckpointManifest {
    CheckpointManifest {
        regions: vec![
            CheckpointRegion {
                name: "first".into(),
                space: 0,
                addr: 0x10000,
                size: 4,
            },
            CheckpointRegion {
                name: "second".into(),
                space: 0,
                addr: 0x20000,
                size: 8,
            },
        ],
    }
}

#[test]
fn a_region_outside_space_zero_is_refused_by_name() {
    let mut manifest = manifest_fixture();
    manifest.regions[1].space = 1;
    match check_manifest(&manifest).expect_err("RPCS3 holds no child space") {
        Rpcs3BridgeError::ChildSpaceRegion { region, space } => {
            assert_eq!(region, "second");
            assert_eq!(space, 1);
        }
        other => panic!("expected ChildSpaceRegion, got {other:?}"),
    }
}

#[test]
fn the_space_field_defaults_to_zero_when_absent() {
    let manifest: CheckpointManifest = toml::from_str(
        r#"
[[regions]]
name = "code"
addr = "0x10000"
size = "0x10"
"#,
    )
    .expect("parses without a space field");
    assert_eq!(manifest.regions[0].space, 0);
    check_manifest(&manifest).expect("space 0 is capturable");
}

#[test]
fn a_negative_space_is_a_parse_error_not_space_zero() {
    let bad = toml::from_str::<CheckpointManifest>(
        r#"
[[regions]]
name = "code"
space = -1
addr = "0x10000"
size = "0x10"
"#,
    );
    assert!(
        bad.is_err(),
        "a negative space names nothing and must not default to the boot space"
    );
}

#[test]
fn dump_slices_contiguously_in_manifest_order() {
    let dump: Vec<u8> = (0..12).collect();
    let manifest = manifest_fixture();
    let obs = build_observation(
        slice_dump(&dump, &manifest).expect("slices"),
        ObservedOutcome::Completed,
        Some(42),
        Decoder::Llvm,
        None,
    );
    assert_eq!(obs.memory_regions.len(), 2);
    assert_eq!(obs.memory_regions[0].name, "first");
    assert_eq!(obs.memory_regions[0].addr, 0x10000);
    assert_eq!(obs.memory_regions[0].data, vec![0, 1, 2, 3]);
    assert_eq!(obs.memory_regions[1].name, "second");
    assert_eq!(obs.memory_regions[1].addr, 0x20000);
    assert_eq!(obs.memory_regions[1].data, vec![4, 5, 6, 7, 8, 9, 10, 11]);
    assert_eq!(obs.metadata.runner, "rpcs3-llvm");
    assert_eq!(obs.metadata.steps, Some(42));
    assert!(obs.state_hashes.is_none());
}

/// `runner` is the only record of which decoder produced a committed
/// observation, so the flag has to reach it.
#[test]
fn the_decoder_reaches_the_runner_field() {
    let dump: Vec<u8> = (0..12).collect();
    let manifest = manifest_fixture();
    for (decoder, expected) in [
        (Decoder::Interpreter, "rpcs3-interpreter"),
        (Decoder::Llvm, "rpcs3-llvm"),
    ] {
        let obs = build_observation(
            slice_dump(&dump, &manifest).expect("slices"),
            ObservedOutcome::Completed,
            None,
            decoder,
            None,
        );
        assert_eq!(obs.metadata.runner, expected);
    }
}

#[test]
fn truncated_dump_is_rejected_with_named_region() {
    let dump: Vec<u8> = vec![0; 10];
    let manifest = manifest_fixture();
    let err = slice_dump(&dump, &manifest).expect_err("truncated");
    match err {
        Rpcs3BridgeError::DumpTruncated { region, .. } => assert_eq!(region, "second"),
        other => panic!("expected DumpTruncated(second), got {other:?}"),
    }
}

#[test]
fn observation_roundtrips_through_json() {
    let dump: Vec<u8> = (0..12).collect();
    let manifest = manifest_fixture();
    let obs = build_observation(
        slice_dump(&dump, &manifest).unwrap(),
        ObservedOutcome::Fault,
        None,
        Decoder::Llvm,
        None,
    );
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
    let m: CheckpointManifest = toml::from_str(&text).expect("manifest parses");
    check_manifest(&m).expect("the committed checkpoint manifest passes the gate");

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
    let m: CheckpointManifest = toml::from_str(toml).unwrap();
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
        rendered.contains("reference-mode config mismatch"),
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
fn hashed_block_carries_the_shared_settings_and_excludes_the_decoders() {
    let hashed = hashed_config_section(REFERENCE_MODE_CONFIG_YAML);
    assert!(hashed.contains("Renderer: \"Null\""), "hashed: {hashed}");
    assert_eq!(
        hashed.matches("Renderer: \"Null\"").count(),
        2,
        "both the video and audio renderer are pinned: {hashed}"
    );
    assert!(
        !hashed.contains("Decoder"),
        "the decoder varies per capture and must stay out of the hash: {hashed}"
    );
}

/// Both decoder pairs stay documented in the file even though neither
/// is hashed; an operator sets RPCS3 from it.
#[test]
fn both_decoder_pairs_are_documented_outside_the_hash() {
    let y = REFERENCE_MODE_CONFIG_YAML;
    for setting in [
        "PPU Decoder: \"Interpreter (static)\"",
        "SPU Decoder: \"Interpreter (static)\"",
        "PPU Decoder: \"Recompiler (LLVM)\"",
        "SPU Decoder: \"Recompiler (LLVM)\"",
    ] {
        assert!(y.contains(setting), "config lost {setting}");
    }
}

/// The whole point of the split: editing the decoder block cannot move
/// the hash, so both variants of a scenario stay reproducible.
#[test]
fn changing_a_decoder_line_does_not_move_the_hash() {
    let swapped = REFERENCE_MODE_CONFIG_YAML.replace(
        "PPU Decoder: \"Recompiler (LLVM)\"",
        "PPU Decoder: \"Interpreter (static)\"",
    );
    assert_ne!(swapped, REFERENCE_MODE_CONFIG_YAML, "the edit applied");
    assert_eq!(
        fnv1a_64(hashed_config_section(&swapped).as_bytes()),
        expected_config_hash()
    );
}

#[test]
fn changing_a_shared_setting_does_move_the_hash() {
    let swapped = REFERENCE_MODE_CONFIG_YAML.replace("Renderer: \"Null\"", "Renderer: \"Vulkan\"");
    assert_ne!(swapped, REFERENCE_MODE_CONFIG_YAML, "the edit applied");
    assert_ne!(
        fnv1a_64(hashed_config_section(&swapped).as_bytes()),
        expected_config_hash()
    );
}

#[test]
fn an_output_named_for_the_other_decoder_is_rejected() {
    use std::path::Path;
    assert!(check_output_names_decoder(
        Path::new("tests/scenario_observations/x/rpcs3_llvm.json"),
        Decoder::Llvm
    )
    .is_ok());
    assert!(check_output_names_decoder(
        Path::new("tests/scenario_observations/x/rpcs3_interpreter.json"),
        Decoder::Interpreter
    )
    .is_ok());
    let err = check_output_names_decoder(
        Path::new("tests/scenario_observations/x/rpcs3_llvm.json"),
        Decoder::Interpreter,
    )
    .expect_err("filename names the other decoder");
    assert!(
        err.to_string().contains("interpreter"),
        "message names the flag: {err}"
    );
    assert!(
        err.to_string().contains("llvm"),
        "message names what the filename claims: {err}"
    );
}

/// The per-title fixture tree writes a fixed `rpcs3/observation.json`
/// that `cellgov dev fixture-gen --rpcs3` reads by that name. A name
/// that claims no decoder cannot file one decoder's answer under the
/// other's name, so the rule has nothing to say about it.
#[test]
fn an_output_naming_no_decoder_is_accepted() {
    use std::path::Path;
    for decoder in [Decoder::Interpreter, Decoder::Llvm] {
        assert!(
            check_output_names_decoder(
                Path::new("tests/fixtures/NPUA80001/rpcs3/observation.json"),
                decoder
            )
            .is_ok(),
            "{} rejected the fixture-tree name",
            decoder.name()
        );
    }
}

/// The bridge spells the runner label itself; `cellgov_compare` spells
/// it again for the live runner. `compare_observations` classifies a
/// step or state-hash difference as a same-runner mismatch or a
/// cross-runner note by testing the two labels for equality, so a
/// drift between the two spellings downgrades a real divergence to a
/// note.
#[test]
fn the_bridge_runner_label_matches_the_live_runners() {
    use cellgov_compare::runner_rpcs3::Rpcs3Decoder;
    let regions = || {
        vec![NamedMemoryRegion {
            name: "r".into(),
            addr: 0,
            data: vec![0],
        }]
    };
    for (bridge, live) in [
        (Decoder::Interpreter, Rpcs3Decoder::Interpreter),
        (Decoder::Llvm, Rpcs3Decoder::Llvm),
    ] {
        let obs = build_observation(regions(), ObservedOutcome::Completed, None, bridge, None);
        assert_eq!(obs.metadata.runner, live.as_runner_str());
    }
}

#[test]
fn a_dump_longer_than_the_manifest_is_rejected() {
    let dump: Vec<u8> = vec![0; 13];
    let manifest = manifest_fixture();
    match slice_dump(&dump, &manifest).expect_err("surplus bytes") {
        Rpcs3BridgeError::DumpLongerThanManifest { declared, dump_len } => {
            assert_eq!(declared, 12);
            assert_eq!(dump_len, 13);
        }
        other => panic!("expected DumpLongerThanManifest, got {other:?}"),
    }
}

#[test]
fn a_manifest_that_repeats_a_region_name_is_rejected() {
    let manifest = CheckpointManifest {
        regions: vec![
            CheckpointRegion {
                name: "result".into(),
                space: 0,
                addr: 0,
                size: 4,
            },
            CheckpointRegion {
                name: "result".into(),
                space: 0,
                addr: 16,
                size: 4,
            },
        ],
    };
    match check_manifest(&manifest).expect_err("repeated name") {
        Rpcs3BridgeError::DuplicateRegionName { region } => assert_eq!(region, "result"),
        other => panic!("expected DuplicateRegionName, got {other:?}"),
    }
}

#[test]
fn a_manifest_with_no_regions_is_rejected() {
    let manifest = CheckpointManifest { regions: vec![] };
    assert!(matches!(
        check_manifest(&manifest).expect_err("nothing to extract"),
        Rpcs3BridgeError::ManifestHasNoRegions
    ));
    assert!(check_manifest(&manifest_fixture()).is_ok());
}

/// The loop assigns straight into a slot, so without the guard the
/// later value wins and the observation's provenance turns on
/// argument order.
#[test]
fn a_flag_given_twice_is_refused_rather_than_overwritten() {
    let argv = |extra: &[&str]| {
        let mut v = vec![
            "rpcs3_to_observation".to_owned(),
            "--dump".to_owned(),
            "d".to_owned(),
            "--manifest".to_owned(),
            "m".to_owned(),
            "--outcome".to_owned(),
            "completed".to_owned(),
            "--output".to_owned(),
            "o.json".to_owned(),
            "--config-hash".to_owned(),
            "0x0".to_owned(),
            "--decoder".to_owned(),
            "llvm".to_owned(),
        ];
        v.extend(extra.iter().map(|s| (*s).to_owned()));
        v
    };
    assert!(parse_args(argv(&[])).is_ok(), "the base argv parses");
    for (flag, value) in [
        ("--decoder", "interpreter"),
        ("--dump", "other"),
        ("--manifest", "other"),
        ("--outcome", "fault"),
        ("--output", "other.json"),
        ("--config-hash", "0x1"),
        ("--steps", "1"),
        ("--rpcs3-dir", "runner"),
    ] {
        // These are absent from the base argv, so pass them twice.
        let extra: Vec<&str> = if matches!(flag, "--steps" | "--rpcs3-dir") {
            vec![flag, value, flag, value]
        } else {
            vec![flag, value]
        };
        match parse_args(argv(&extra)) {
            Err(Rpcs3BridgeError::DuplicateFlag { flag: named }) => assert_eq!(named, flag),
            Err(other) => panic!("expected DuplicateFlag for {flag}, got {other:?}"),
            Ok(_) => panic!("{flag} twice was accepted"),
        }
    }
}

/// `--dump` and `--tty` name different captures, not a repeat of one.
#[test]
fn two_capture_sources_are_still_the_ambiguity_error_not_a_duplicate_flag() {
    let argv: Vec<String> = [
        "rpcs3_to_observation",
        "--dump",
        "d",
        "--tty",
        "t",
        "--manifest",
        "m",
        "--outcome",
        "completed",
        "--output",
        "o.json",
        "--config-hash",
        "0x0",
        "--decoder",
        "llvm",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    match parse_args(argv) {
        Err(Rpcs3BridgeError::CaptureSourceAmbiguous) => {}
        Err(other) => panic!("expected CaptureSourceAmbiguous, got {other:?}"),
        Ok(_) => panic!("two capture sources were accepted"),
    }
}

#[test]
fn decoder_token_round_trips_and_rejects_others() {
    assert_eq!(parse_decoder("interpreter").unwrap(), Decoder::Interpreter);
    assert_eq!(parse_decoder("llvm").unwrap(), Decoder::Llvm);
    assert_eq!(parse_decoder("interpreter").unwrap().name(), "interpreter");
    assert_eq!(parse_decoder("llvm").unwrap().name(), "llvm");
    let err = parse_decoder("asmjit").expect_err("unknown decoder");
    assert!(err.to_string().contains("interpreter, llvm"), "{err}");
}
