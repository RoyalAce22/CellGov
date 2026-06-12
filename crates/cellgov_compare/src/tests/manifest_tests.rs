//! Manifest TOML parsing: full and section-omitted forms, field defaults,
//! outcome-field isomorphism, and the checked-in micro-test manifests.

use super::*;
use strum::VariantArray;

const FULL_MANIFEST: &str = r#"
[test]
name = "mailbox_roundtrip_001"

[cellgov]
scenario = "mailbox_roundtrip"
scenario_args = { command = 66 }
max_steps = 10000

[rpcs3]
binary = "tests/micro/mailbox_roundtrip/spu.elf"
decoder = "interpreter"
timeout_ms = 5000

[observe]
memory_regions = [
  { name = "result", addr = 65536, size = 64 },
]
mailbox_sequences = true
final_hashes = true
event_classes = ["mailbox", "dma", "wakeup"]

[expect]
outcome = "completed"
"#;

#[test]
fn parse_full_manifest() {
    let m = parse(FULL_MANIFEST).expect("parse");
    assert_eq!(m.test.name, "mailbox_roundtrip_001");

    let cg = m.cellgov.unwrap();
    assert_eq!(cg.scenario, "mailbox_roundtrip");
    assert_eq!(cg.max_steps, 10000);
    assert_eq!(
        cg.scenario_args.get("command").and_then(|v| v.as_integer()),
        Some(66)
    );

    let rpcs3 = m.rpcs3.unwrap();
    assert_eq!(rpcs3.binary, "tests/micro/mailbox_roundtrip/spu.elf");
    assert!(matches!(rpcs3.decoder, DecoderField::Interpreter));
    assert_eq!(rpcs3.timeout_ms, 5000);

    assert_eq!(m.observe.memory_regions.len(), 1);
    assert_eq!(m.observe.memory_regions[0].name, "result");
    assert_eq!(m.observe.memory_regions[0].addr, 65536);
    assert_eq!(m.observe.memory_regions[0].size, 64);
    assert!(m.observe.mailbox_sequences);
    assert!(m.observe.final_hashes);
    assert_eq!(m.observe.event_classes, vec!["mailbox", "dma", "wakeup"]);

    assert!(matches!(m.expect.outcome, OutcomeField::Completed));
}

#[test]
fn parse_cellgov_only_manifest() {
    let text = r#"
[test]
name = "cellgov_only"

[cellgov]
scenario = "fairness"

[observe]
memory_regions = []

[expect]
outcome = "completed"
"#;
    let m = parse(text).expect("parse");
    assert!(m.cellgov.is_some());
    assert!(m.rpcs3.is_none());
    assert_eq!(m.cellgov.unwrap().max_steps, 10000);
}

#[test]
fn parse_rpcs3_only_manifest() {
    let text = r#"
[test]
name = "rpcs3_only"

[rpcs3]
binary = "test.elf"
decoder = "llvm"
timeout_ms = 3000

[observe]
memory_regions = []

[expect]
outcome = "completed"
"#;
    let m = parse(text).expect("parse");
    assert!(m.cellgov.is_none());
    assert!(m.rpcs3.is_some());
    let rpcs3 = m.rpcs3.unwrap();
    assert!(matches!(rpcs3.decoder, DecoderField::Llvm));
    assert_eq!(rpcs3.timeout_ms, 3000);
}

#[test]
fn defaults_apply_when_fields_omitted() {
    let text = r#"
[test]
name = "defaults"

[cellgov]
scenario = "isa"

[rpcs3]
binary = "test.elf"

[observe]

[expect]
outcome = "completed"
"#;
    let m = parse(text).expect("parse");
    assert_eq!(m.cellgov.unwrap().max_steps, 10000);
    let rpcs3 = m.rpcs3.unwrap();
    assert!(matches!(rpcs3.decoder, DecoderField::Interpreter));
    assert_eq!(rpcs3.timeout_ms, 5000);
    assert!(m.observe.memory_regions.is_empty());
    assert!(!m.observe.mailbox_sequences);
    assert!(!m.observe.final_hashes);
}

#[test]
fn all_outcome_variants_parse() {
    for (text, expected) in [
        ("completed", "Completed"),
        ("stalled", "Stalled"),
        ("timeout", "Timeout"),
        ("fault", "Fault"),
    ] {
        let toml_text = format!(
            r#"
[test]
name = "outcome_test"
[observe]
[expect]
outcome = "{text}"
"#
        );
        let m = parse(&toml_text).expect("parse");
        let mapped: ObservedOutcome = m.expect.outcome.into();
        assert_eq!(format!("{mapped:?}"), expected);
    }
}

#[test]
fn invalid_toml_returns_error() {
    let result = parse("not valid toml {{{}}}");
    assert!(result.is_err());
}

#[test]
fn missing_required_field_returns_error() {
    let result = parse("[test]\n[observe]\n[expect]\noutcome = \"completed\"");
    assert!(result.is_err());
}

#[test]
fn outcome_field_and_observed_outcome_are_isomorphic() {
    let field_count = OutcomeField::VARIANTS.len();
    let observed_count = ObservedOutcome::VARIANTS.len();
    assert_eq!(
        field_count, observed_count,
        "OutcomeField has {field_count} variants but ObservedOutcome has {observed_count}; \
         add the missing variant on both sides and a `From` arm",
    );
    for f in OutcomeField::VARIANTS {
        let observed: ObservedOutcome = f.clone().into();
        assert!(
            ObservedOutcome::VARIANTS.contains(&observed),
            "OutcomeField::{f:?} -> {observed:?} is not in ObservedOutcome::VARIANTS",
        );
    }
}

#[test]
fn process_exit_accepts_all_three_spellings() {
    let canonical: OutcomeField = toml::from_str(r#"value = "processexit""#)
        .map(|t: toml::Table| t["value"].clone())
        .and_then(|v| v.try_into())
        .expect("canonical lowercase spelling parses");
    let snake: OutcomeField = toml::from_str(r#"value = "process_exit""#)
        .map(|t: toml::Table| t["value"].clone())
        .and_then(|v| v.try_into())
        .expect("snake_case alias parses");
    let kebab: OutcomeField = toml::from_str(r#"value = "process-exit""#)
        .map(|t: toml::Table| t["value"].clone())
        .and_then(|v| v.try_into())
        .expect("kebab-case alias parses");
    assert_eq!(canonical, OutcomeField::ProcessExit);
    assert_eq!(snake, OutcomeField::ProcessExit);
    assert_eq!(kebab, OutcomeField::ProcessExit);
}

#[test]
fn load_spu_mailbox_write_manifest() {
    let path = std::path::Path::new("../../tests/micro/spu_mailbox_write/manifest.toml");
    if path.exists() {
        let m = load(path).expect("load manifest");
        assert_eq!(m.test.name, "spu_mailbox_write");
        assert!(m.cellgov.is_some());
        assert!(m.rpcs3.is_none());
    }
}

#[test]
fn load_spu_fixed_value_manifest() {
    let path = std::path::Path::new("../../tests/micro/spu_fixed_value/manifest.toml");
    if path.exists() {
        let m = load(path).expect("load manifest");
        assert_eq!(m.test.name, "spu_fixed_value");
        assert!(m.cellgov.is_none());
        assert!(m.rpcs3.is_some());
        let rpcs3 = m.rpcs3.unwrap();
        assert!(matches!(rpcs3.decoder, DecoderField::Interpreter));
        assert_eq!(m.observe.memory_regions.len(), 1);
        assert_eq!(m.observe.memory_regions[0].name, "result");
        assert_eq!(m.observe.memory_regions[0].size, 8);
    }
}

#[test]
fn load_atomic_reservation_manifest() {
    let path = std::path::Path::new("../../tests/micro/atomic_reservation/manifest.toml");
    if path.exists() {
        let m = load(path).expect("load manifest");
        assert_eq!(m.test.name, "atomic_reservation");
        assert!(m.cellgov.is_none());
        assert!(m.rpcs3.is_some());
        let rpcs3 = m.rpcs3.unwrap();
        assert!(matches!(rpcs3.decoder, DecoderField::Interpreter));
        assert_eq!(m.observe.memory_regions.len(), 2);
        assert_eq!(m.observe.memory_regions[0].name, "header");
        assert_eq!(m.observe.memory_regions[0].size, 8);
        assert_eq!(m.observe.memory_regions[1].name, "data");
        assert_eq!(m.observe.memory_regions[1].size, 128);
    }
}

#[test]
fn load_ls_to_shared_manifest() {
    let path = std::path::Path::new("../../tests/micro/ls_to_shared/manifest.toml");
    if path.exists() {
        let m = load(path).expect("load manifest");
        assert_eq!(m.test.name, "ls_to_shared");
        assert!(m.cellgov.is_none());
        assert!(m.rpcs3.is_some());
        let rpcs3 = m.rpcs3.unwrap();
        assert!(matches!(rpcs3.decoder, DecoderField::Interpreter));
        assert_eq!(m.observe.memory_regions.len(), 2);
        assert_eq!(m.observe.memory_regions[0].name, "header");
        assert_eq!(m.observe.memory_regions[0].size, 8);
        assert_eq!(m.observe.memory_regions[1].name, "data");
        assert_eq!(m.observe.memory_regions[1].size, 128);
    }
}

#[test]
fn load_barrier_wakeup_manifest() {
    let path = std::path::Path::new("../../tests/micro/barrier_wakeup/manifest.toml");
    if path.exists() {
        let m = load(path).expect("load manifest");
        assert_eq!(m.test.name, "barrier_wakeup");
        assert!(m.cellgov.is_none());
        assert!(m.rpcs3.is_some());
        let rpcs3 = m.rpcs3.unwrap();
        assert!(matches!(rpcs3.decoder, DecoderField::Interpreter));
        assert_eq!(m.observe.memory_regions.len(), 2);
        assert_eq!(m.observe.memory_regions[0].name, "spu0_result");
        assert_eq!(m.observe.memory_regions[0].size, 8);
        assert_eq!(m.observe.memory_regions[1].name, "spu1_result");
        assert_eq!(m.observe.memory_regions[1].size, 8);
    }
}
