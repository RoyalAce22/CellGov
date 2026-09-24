//! Boot stop-condition and its CLI parser.

/// Why parsing a `--checkpoint` argument or manifest `pc = "..."` failed.
#[derive(Debug, thiserror::Error)]
pub enum CheckpointParseError {
    /// Unknown checkpoint kind keyword.
    #[error("unknown checkpoint kind '{0}' (accepted: process-exit, first-rsx-write, pc=0xADDR)")]
    UnknownKind(String),
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

    /// The wire form an anchor records this stop condition in.
    pub fn kind(self) -> cellgov_compare::CheckpointKind {
        use cellgov_compare::CheckpointKind;
        match self {
            Self::ProcessExit => CheckpointKind::ProcessExit,
            Self::FirstRsxWrite => CheckpointKind::FirstRsxWrite,
            Self::Pc(addr) => CheckpointKind::Pc {
                addr: cellgov_mem::GuestAddr::new(addr),
            },
        }
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
#[path = "tests/checkpoint_tests.rs"]
mod tests;
