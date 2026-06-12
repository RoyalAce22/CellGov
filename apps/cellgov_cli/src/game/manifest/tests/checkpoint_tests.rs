//! Checkpoint-trigger parsing from CLI values and argument lists.

use super::*;

#[test]
fn checkpoint_parse_cli_forms() {
    assert_eq!(
        CheckpointTrigger::parse_cli_value("process-exit").unwrap(),
        CheckpointTrigger::ProcessExit
    );
    assert_eq!(
        CheckpointTrigger::parse_cli_value("first-rsx-write").unwrap(),
        CheckpointTrigger::FirstRsxWrite
    );
    assert_eq!(
        CheckpointTrigger::parse_cli_value("pc=0x10381ce8").unwrap(),
        CheckpointTrigger::Pc(0x10381ce8)
    );
    assert!(CheckpointTrigger::parse_cli_value("nope").is_err());
    assert!(CheckpointTrigger::parse_cli_value("pc=xyz").is_err());
}

#[test]
fn checkpoint_unprefixed_digits_parse_as_decimal_not_hex() {
    assert_eq!(
        CheckpointTrigger::parse_cli_value("pc=10").unwrap(),
        CheckpointTrigger::Pc(10)
    );
}

#[test]
fn checkpoint_unprefixed_hex_is_rejected() {
    assert!(CheckpointTrigger::parse_cli_value("pc=1ce8").is_err());
}

#[test]
fn parse_from_args_rejects_repeated_flag() {
    let args = vec![
        "run-game".to_string(),
        "--checkpoint".to_string(),
        "process-exit".to_string(),
        "--checkpoint".to_string(),
        "first-rsx-write".to_string(),
    ];
    let got = CheckpointTrigger::parse_from_args(&args);
    assert!(
        matches!(got, Some(Err(_))),
        "repeated --checkpoint must surface as Some(Err)"
    );
}

#[test]
fn parse_from_args_rejects_missing_value() {
    let args = vec!["run-game".to_string(), "--checkpoint".to_string()];
    let got = CheckpointTrigger::parse_from_args(&args);
    assert!(
        matches!(got, Some(Err(_))),
        "--checkpoint with no value must be Some(Err), not None"
    );
}

#[test]
fn parse_from_args_returns_none_when_flag_absent() {
    let args = vec!["run-game".to_string(), "--other".to_string()];
    assert!(CheckpointTrigger::parse_from_args(&args).is_none());
}

// Same PC used in `boot_summary_cross_check` in observation.rs;
// grep `0x10381ce8` to find all wire-form pins.
const SAMPLE_PC: u64 = 0x10381ce8;

fn wire_form_cases() -> [CheckpointTrigger; 3] {
    // Compile-break on a new variant so the array stays in sync.
    match CheckpointTrigger::ProcessExit {
        CheckpointTrigger::ProcessExit
        | CheckpointTrigger::FirstRsxWrite
        | CheckpointTrigger::Pc(_) => {}
    }
    [
        CheckpointTrigger::ProcessExit,
        CheckpointTrigger::FirstRsxWrite,
        CheckpointTrigger::Pc(SAMPLE_PC),
    ]
}

#[test]
fn cli_wire_form_round_trips() {
    for case in wire_form_cases() {
        let s = case.as_cli_str();
        let back = CheckpointTrigger::parse_cli_value(&s).expect("emitted form must round-trip");
        assert_eq!(back, case, "round-trip failed for {case:?} via {s:?}");
    }
}

#[test]
fn cli_wire_form_emits_expected_strings() {
    assert_eq!(CheckpointTrigger::ProcessExit.as_cli_str(), "process-exit");
    assert_eq!(
        CheckpointTrigger::FirstRsxWrite.as_cli_str(),
        "first-rsx-write"
    );
    assert_eq!(
        CheckpointTrigger::Pc(SAMPLE_PC).as_cli_str(),
        "pc=0x10381ce8"
    );
}

#[test]
fn markdown_wire_form_emits_expected_strings() {
    // Pin the markdown form (PascalCase) against the CLI form
    // (kebab) so the two cannot drift apart silently.
    let cli_to_kind = |c: CheckpointTrigger| -> cellgov_compare::CheckpointKind {
        match c {
            CheckpointTrigger::ProcessExit => cellgov_compare::CheckpointKind::ProcessExit,
            CheckpointTrigger::FirstRsxWrite => cellgov_compare::CheckpointKind::FirstRsxWrite,
            CheckpointTrigger::Pc(addr) => cellgov_compare::CheckpointKind::Pc {
                addr: cellgov_mem::GuestAddr::new(addr),
            },
        }
    };
    assert_eq!(
        cli_to_kind(CheckpointTrigger::ProcessExit).as_markdown_label(),
        "ProcessExit"
    );
    assert_eq!(
        cli_to_kind(CheckpointTrigger::FirstRsxWrite).as_markdown_label(),
        "FirstRsxWrite"
    );
    assert_eq!(
        cli_to_kind(CheckpointTrigger::Pc(SAMPLE_PC)).as_markdown_label(),
        "Pc=0x10381ce8"
    );
}

#[test]
fn all_three_wire_forms_cover_every_variant() {
    // Pin that wire forms are pairwise-distinct across variants;
    // two variants must not collapse to the same string.
    let mut cli_seen = std::collections::BTreeSet::new();
    let mut md_seen = std::collections::BTreeSet::new();
    for case in wire_form_cases() {
        let cli = case.as_cli_str();
        assert!(!cli.is_empty(), "CLI form empty for {case:?}");
        assert!(
            cli_seen.insert(cli.clone()),
            "CLI form collision: {cli:?} appears for two variants"
        );

        let kind = match case {
            CheckpointTrigger::ProcessExit => cellgov_compare::CheckpointKind::ProcessExit,
            CheckpointTrigger::FirstRsxWrite => cellgov_compare::CheckpointKind::FirstRsxWrite,
            CheckpointTrigger::Pc(addr) => cellgov_compare::CheckpointKind::Pc {
                addr: cellgov_mem::GuestAddr::new(addr),
            },
        };
        let md = kind.as_markdown_label();
        assert!(!md.is_empty(), "markdown form empty for {case:?}");
        assert!(
            md_seen.insert(md.clone()),
            "markdown form collision: {md:?} appears for two variants"
        );
    }
}
