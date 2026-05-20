//! Boot stop-condition and its CLI parser.

/// Why parsing a `--checkpoint` argument or manifest `pc = "..."` failed.
#[derive(Debug)]
pub enum CheckpointParseError {
    /// Unknown checkpoint kind keyword.
    UnknownKind(String),
    /// `--checkpoint` was specified more than once.
    RepeatedFlag,
    /// `--checkpoint` had no following value.
    MissingValue,
    /// `pc=` value has `0x`/`0X` prefix but is not valid hex u64.
    PcNotHex(String),
    /// `pc=` value has no hex prefix and is not a valid decimal u64.
    PcNotDecimal(String),
}

impl std::fmt::Display for CheckpointParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownKind(v) => write!(
                f,
                "unknown checkpoint kind '{v}' (accepted: \
                 process-exit, first-rsx-write, pc=0xADDR)"
            ),
            Self::RepeatedFlag => {
                f.write_str("--checkpoint was specified more than once; pass it exactly once.")
            }
            Self::MissingValue => f.write_str(
                "--checkpoint requires a value (process-exit, first-rsx-write, \
                 or pc=0xADDR)",
            ),
            Self::PcNotHex(raw) => write!(f, "checkpoint pc value '{raw}' is not a hex u64"),
            Self::PcNotDecimal(raw) => write!(
                f,
                "checkpoint pc value '{raw}' is not a decimal u64 (use 0x prefix for hex)"
            ),
        }
    }
}

impl std::error::Error for CheckpointParseError {}

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
            // Skip the value token so a second `--checkpoint` used as
            // a value is not rematched as the flag.
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
}
