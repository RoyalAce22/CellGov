//! Subcommand token dispatch and step-count parsing.

use super::*;

#[test]
fn parse_step_count_accepts_decimal() {
    assert_eq!(parse_step_count("123").unwrap(), 123);
}

#[test]
fn parse_step_count_accepts_lower_hex_prefix() {
    assert_eq!(parse_step_count("0xff").unwrap(), 0xff);
}

#[test]
fn parse_step_count_accepts_upper_hex_prefix() {
    assert_eq!(parse_step_count("0XFF").unwrap(), 0xff);
}

#[test]
fn parse_step_count_rejects_garbage() {
    assert!(parse_step_count("nope").is_err());
    assert!(parse_step_count("0xnope").is_err());
}

#[test]
fn subcommands_const_is_exhaustive() {
    for sub in SUBCOMMANDS {
        for tok in sub.tokens() {
            assert_eq!(
                Subcommand::from_token(tok),
                Some(*sub),
                "token {tok:?} did not round-trip to {sub:?}"
            );
        }
    }
    let expected: &[Subcommand] = &[
        Subcommand::Help,
        Subcommand::Version,
        Subcommand::Compare,
        Subcommand::CompareObservations,
        Subcommand::Diverge,
        Subcommand::Zoom,
        Subcommand::Explore,
        Subcommand::RunGame,
        Subcommand::BenchBoot,
        Subcommand::BenchBootOnce,
        Subcommand::Dump,
        Subcommand::DumpPrxImports,
        Subcommand::Disasm,
        Subcommand::Rpcs3Attribute,
        Subcommand::FixtureGen,
        Subcommand::TitlesGen,
    ];
    assert_eq!(
        SUBCOMMANDS.len(),
        expected.len(),
        "SUBCOMMANDS missing a variant present in `expected`"
    );
    for (a, b) in SUBCOMMANDS.iter().zip(expected.iter()) {
        assert_eq!(a, b, "SUBCOMMANDS ordering drifted from `expected`");
    }
}

#[test]
fn tokens_have_no_duplicates_across_variants() {
    let mut seen: std::collections::BTreeMap<&str, Subcommand> = std::collections::BTreeMap::new();
    for sub in SUBCOMMANDS {
        for tok in sub.tokens() {
            if let Some(prev) = seen.insert(*tok, *sub) {
                panic!(
                    "token {tok:?} claimed by both {prev:?} and {sub:?}",
                    prev = prev,
                    sub = sub
                );
            }
        }
    }
}

#[test]
fn scenarios_disjoint_from_subcommand_tokens() {
    for s in SCENARIOS {
        assert!(
            Subcommand::from_token(s).is_none(),
            "scenario {s:?} shadowed by dispatcher token"
        );
    }
}

#[test]
fn from_token_returns_none_for_unknown() {
    assert_eq!(Subcommand::from_token(""), None);
    assert_eq!(Subcommand::from_token("nope"), None);
    assert_eq!(Subcommand::from_token("compaer"), None);
}
