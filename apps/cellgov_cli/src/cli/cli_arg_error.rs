//! Typed error for the CLI argument-parsing layer.
//!
//! Inner `_inner` parsers in `cli/args.rs` and `cli/env.rs` return
//! `Result<_, CliArgError>`. The outer wrappers call `die()` on the
//! Display rendering, so the user-visible message is unchanged.

/// Why a CLI argument or environment-variable parser rejected its input.
#[derive(Debug)]
pub(crate) enum CliArgError {
    /// Hex literal is empty after trimming whitespace.
    EmptyHexValue { context: String },
    /// `0x` / `0X` prefix with no following digits.
    HexPrefixNoDigits { context: String, raw: String },
    /// Hex parse via `u64::from_str_radix` failed.
    CannotParseHexU64 {
        context: String,
        raw: String,
        source: std::num::ParseIntError,
    },
    /// Hex parse via `u8::from_str_radix` failed.
    CannotParseHexU8 {
        context: String,
        raw: String,
        source: std::num::ParseIntError,
    },
    /// Hex u8 literal is longer than 2 digits.
    HexU8TooLong {
        context: String,
        raw: String,
        digits: usize,
    },
    /// `--flag` was specified more than once.
    DuplicateFlag { flag: String },
    /// `--flag` has no following value.
    FlagRequiresValue { flag: String },
    /// `--flag=value` form used where the two-token form is required.
    FlagEqNotSupported { flag: String },
    /// `--flag value` where `value` itself starts with `--`.
    FlagValueLooksLikeFlag { flag: String, value: String },
    /// `--format <kind>` got an unknown kind.
    UnknownOutputFormat { got: String },
    /// `--mode <kind>` got an unknown kind.
    UnknownCompareMode { got: String },
    /// `--patch-byte ""`.
    PatchByteEmpty,
    /// `--patch-byte` value lacks `=`.
    PatchByteMissingEq { pair: String },
    /// `--patch-byte =VALUE`.
    PatchByteEmptyAddress { pair: String },
    /// `--patch-byte ADDR=`.
    PatchByteEmptyValue { pair: String },
    /// `--patch-byte ADDR=VAL=EXTRA`.
    PatchByteExtraEq { pair: String },
    /// run-game positional: a second non-flag-non-value token.
    ExtraPositional { value: String, existing: String },
    /// run-game positional: empty argv slot.
    EmptyPositional,
    /// CELLGOV_* env-var value not a recognized boolean.
    EnvBoolUnknown { name: String, got: String },
}

impl std::fmt::Display for CliArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyHexValue { context } => write!(f, "{context}: empty hex value"),
            Self::HexPrefixNoDigits { context, raw } => {
                write!(f, "{context}: hex prefix with no digits in {raw:?}")
            }
            Self::CannotParseHexU64 {
                context,
                raw,
                source,
            } => write!(f, "{context}: cannot parse hex {raw:?}: {source}"),
            Self::CannotParseHexU8 {
                context,
                raw,
                source,
            } => write!(f, "{context}: cannot parse hex u8 {raw:?}: {source}"),
            Self::HexU8TooLong {
                context,
                raw,
                digits,
            } => write!(
                f,
                "{context}: expected 1-2 hex digits, got {digits} in {raw:?}"
            ),
            Self::DuplicateFlag { flag } => write!(
                f,
                "{flag} was specified more than once; pass it exactly once"
            ),
            Self::FlagRequiresValue { flag } => write!(f, "{flag} requires a value"),
            Self::FlagEqNotSupported { flag } => write!(
                f,
                "{flag}=... not supported; use the two-token form: {flag} <value>"
            ),
            Self::FlagValueLooksLikeFlag { flag, value } => write!(
                f,
                "{flag} expects a value but got flag-like token {value:?}; \
                 likely a missing value upstream"
            ),
            Self::UnknownOutputFormat { got } => write!(
                f,
                "unknown output format: {got}\nvalid formats: human, json"
            ),
            Self::UnknownCompareMode { got } => write!(
                f,
                "unknown compare mode: {got}\nvalid modes: strict, memory, events, prefix"
            ),
            Self::PatchByteEmpty => {
                f.write_str("--patch-byte: empty argument (expected ADDR=VALUE)")
            }
            Self::PatchByteMissingEq { pair } => write!(
                f,
                "--patch-byte: missing '=' in {pair:?} (expected ADDR=VALUE)"
            ),
            Self::PatchByteEmptyAddress { pair } => write!(
                f,
                "--patch-byte: empty address in {pair:?} (expected ADDR=VALUE)"
            ),
            Self::PatchByteEmptyValue { pair } => write!(
                f,
                "--patch-byte: empty value in {pair:?} (expected ADDR=VALUE)"
            ),
            Self::PatchByteExtraEq { pair } => {
                write!(
                    f,
                    "--patch-byte: extra '=' in {pair:?} (expected ADDR=VALUE)"
                )
            }
            Self::ExtraPositional { value, existing } => write!(
                f,
                "unexpected extra positional: {value:?} (already have {existing:?})"
            ),
            Self::EmptyPositional => f.write_str("unexpected empty positional argument"),
            Self::EnvBoolUnknown { name, got } => {
                write!(f, "{name}={got:?}: expected 0/1/true/false/yes/no/on/off")
            }
        }
    }
}

impl std::error::Error for CliArgError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CannotParseHexU64 { source, .. } | Self::CannotParseHexU8 { source, .. } => {
                Some(source)
            }
            _ => None,
        }
    }
}
