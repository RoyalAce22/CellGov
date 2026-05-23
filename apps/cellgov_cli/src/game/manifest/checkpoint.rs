//! Boot stop-condition and its CLI parser.

/// Why parsing a `--checkpoint` argument or manifest `pc = "..."` failed.
#[derive(Debug, thiserror::Error)]
pub enum CheckpointParseError {
    /// Unknown checkpoint kind keyword.
    #[error("unknown checkpoint kind '{0}' (accepted: process-exit, first-rsx-write, pc=0xADDR)")]
    UnknownKind(String),
    /// `--checkpoint` was specified more than once.
    #[error("--checkpoint was specified more than once; pass it exactly once.")]
    RepeatedFlag,
    /// `--checkpoint` had no following value.
    #[error("--checkpoint requires a value (process-exit, first-rsx-write, or pc=0xADDR)")]
    MissingValue,
    /// `pc=` value has `0x`/`0X` prefix but is not valid hex u64.
    #[error("checkpoint pc value '{0}' is not a hex u64")]
    PcNotHex(String),
    /// `pc=` value has no hex prefix and is not a valid decimal u64.
    #[error("checkpoint pc value '{0}' is not a decimal u64 (use 0x prefix for hex)")]
    PcNotDecimal(String),
}

/// Stop condition for a boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointTrigger {
    /// Stop on `sys_process_exit`.
    ProcessExit,
    /// Stop on the first PPU write into the RSX region; the resulting
    /// `MemError::ReservedWrite { region: "rsx", .. }` is classified as
    /// "checkpoint reached", not a fault.
    FirstRsxWrite,
    /// Stop when a step retires at the given guest PC.
    Pc(u64),
}

impl CheckpointTrigger {
    /// Accepts `process-exit`, `first-rsx-write`, `pc=0xHEX`, or
    /// `pc=DECIMAL`. Hex requires the `0x`/`0X` prefix.
    pub fn parse_cli_value(value: &str) -> Result<Self, CheckpointParseError> {
        match value {
            "process-exit" => Ok(Self::ProcessExit),
            "first-rsx-write" => Ok(Self::FirstRsxWrite),
            _ => {
                if let Some(rest) = value.strip_prefix("pc=") {
                    parse_pc_literal(rest).map(Self::Pc)
                } else {
                    Err(CheckpointParseError::UnknownKind(value.to_string()))
                }
            }
        }
    }

    /// Inverse of [`Self::parse_cli_value`]: `"process-exit"`,
    /// `"first-rsx-write"`, or `"pc=0xADDR"`.
    pub fn as_cli_str(&self) -> String {
        match self {
            Self::ProcessExit => "process-exit".to_string(),
            Self::FirstRsxWrite => "first-rsx-write".to_string(),
            Self::Pc(addr) => format!("pc=0x{addr:x}"),
        }
    }

    /// `None` means the flag was absent; `Some(Err)` covers malformed,
    /// repeated, or value-missing cases.
    pub fn parse_from_args(args: &[String]) -> Option<Result<Self, CheckpointParseError>> {
        let mut found: Option<Result<Self, CheckpointParseError>> = None;
        let mut i = 0;
        while i < args.len() {
            if args[i] != "--checkpoint" {
                i += 1;
                continue;
            }
            if found.is_some() {
                return Some(Err(CheckpointParseError::RepeatedFlag));
            }
            let parsed = match args.get(i + 1) {
                Some(v) => Self::parse_cli_value(v.as_str()),
                None => Err(CheckpointParseError::MissingValue),
            };
            found = Some(parsed);
            // Skip past the value so it cannot rematch as a flag.
            i += 2;
        }
        found
    }
}

/// Shared between `--checkpoint pc=...` and manifest `pc = "..."`.
/// Hex requires `0x`/`0X`; otherwise decimal.
pub(super) fn parse_pc_literal(raw: &str) -> Result<u64, CheckpointParseError> {
    if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).map_err(|_| CheckpointParseError::PcNotHex(raw.to_string()))
    } else {
        raw.parse::<u64>()
            .map_err(|_| CheckpointParseError::PcNotDecimal(raw.to_string()))
    }
}

#[cfg(test)]
mod tests {
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
            let back =
                CheckpointTrigger::parse_cli_value(&s).expect("emitted form must round-trip");
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
}
