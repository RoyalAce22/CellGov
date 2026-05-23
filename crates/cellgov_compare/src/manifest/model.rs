//! [`Manifest`] -- the top-level manifest struct. Field types and
//! section structs live in [`super::fields`] and [`super::sections`].

use serde::Deserialize;

use crate::manifest::sections::{
    CellGovSection, ExpectSection, ObserveSection, Rpcs3Section, TestSection,
};

/// A parsed microtest manifest.
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    /// Test identity.
    pub test: TestSection,
    /// CellGov-side configuration (absent for RPCS3-only tests).
    pub cellgov: Option<CellGovSection>,
    /// RPCS3-side configuration (absent for CellGov-only tests).
    pub rpcs3: Option<Rpcs3Section>,
    /// What to observe and compare.
    pub observe: ObserveSection,
    /// Expected outcome.
    pub expect: ExpectSection,
}

#[cfg(test)]
mod tests {
    use crate::manifest::fields::{DecoderField, OutcomeField};
    use crate::manifest::parse;
    use crate::observation::ObservedOutcome;

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
}
