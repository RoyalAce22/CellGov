//! Command-line argument parsing primitives over `&[String]`. Each
//! parser pairs a public wrapper that calls [`die`] on malformed
//! input with a private `_inner` variant returning
//! `Result<_, CliArgError>` for testability.

use cellgov_compare::CompareMode;

use super::exit::die;

/// Why a CLI argument or environment-variable parser rejected its input.
///
/// Inner `_inner` parsers in `cli/args.rs` and `cli/env.rs` return
/// `Result<_, CliArgError>`. The outer wrappers call `die()` on the
/// Display rendering, so the user-visible message is unchanged.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CliArgError {
    /// Hex literal is empty after trimming whitespace.
    #[error("{context}: empty hex value")]
    EmptyHexValue { context: String },
    /// `0x` / `0X` prefix with no following digits.
    #[error("{context}: hex prefix with no digits in {raw:?}")]
    HexPrefixNoDigits { context: String, raw: String },
    /// Hex parse via `u64::from_str_radix` failed.
    #[error("{context}: cannot parse hex {raw:?}: {source}")]
    CannotParseHexU64 {
        context: String,
        raw: String,
        #[source]
        source: std::num::ParseIntError,
    },
    /// Hex parse via `u8::from_str_radix` failed.
    #[error("{context}: cannot parse hex u8 {raw:?}: {source}")]
    CannotParseHexU8 {
        context: String,
        raw: String,
        #[source]
        source: std::num::ParseIntError,
    },
    /// Hex u8 literal is longer than 2 digits.
    #[error("{context}: expected 1-2 hex digits, got {digits} in {raw:?}")]
    HexU8TooLong {
        context: String,
        raw: String,
        digits: usize,
    },
    /// `--flag` was specified more than once.
    #[error("{flag} was specified more than once; pass it exactly once")]
    DuplicateFlag { flag: String },
    /// `--flag` has no following value.
    #[error("{flag} requires a value")]
    FlagRequiresValue { flag: String },
    /// `--flag=value` form used where the two-token form is required.
    #[error("{flag}=... not supported; use the two-token form: {flag} <value>")]
    FlagEqNotSupported { flag: String },
    /// `--flag value` where `value` itself starts with `--`.
    #[error(
        "{flag} expects a value but got flag-like token {value:?}; likely a missing value upstream"
    )]
    FlagValueLooksLikeFlag { flag: String, value: String },
    /// `--format <kind>` got an unknown kind.
    #[error("unknown output format: {got}\nvalid formats: human, json")]
    UnknownOutputFormat { got: String },
    /// `--mode <kind>` got an unknown kind.
    #[error("unknown compare mode: {got}\nvalid modes: strict, memory, events, prefix")]
    UnknownCompareMode { got: String },
    /// `--patch-byte ""`.
    #[error("--patch-byte: empty argument (expected ADDR=VALUE)")]
    PatchByteEmpty,
    /// `--patch-byte` value lacks `=`.
    #[error("--patch-byte: missing '=' in {pair:?} (expected ADDR=VALUE)")]
    PatchByteMissingEq { pair: String },
    /// `--patch-byte =VALUE`.
    #[error("--patch-byte: empty address in {pair:?} (expected ADDR=VALUE)")]
    PatchByteEmptyAddress { pair: String },
    /// `--patch-byte ADDR=`.
    #[error("--patch-byte: empty value in {pair:?} (expected ADDR=VALUE)")]
    PatchByteEmptyValue { pair: String },
    /// `--patch-byte ADDR=VAL=EXTRA`.
    #[error("--patch-byte: extra '=' in {pair:?} (expected ADDR=VALUE)")]
    PatchByteExtraEq { pair: String },
    /// run-game positional: a second non-flag-non-value token.
    #[error("unexpected extra positional: {value:?} (already have {existing:?})")]
    ExtraPositional { value: String, existing: String },
    /// run-game positional: empty argv slot.
    #[error("unexpected empty positional argument")]
    EmptyPositional,
    /// CELLGOV_* env-var value not a recognized boolean.
    #[error("{name}={got:?}: expected 0/1/true/false/yes/no/on/off")]
    EnvBoolUnknown { name: String, got: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Human,
    Json,
}

pub(crate) fn parse_output_format(args: &[String]) -> OutputFormat {
    parse_output_format_inner(args).unwrap_or_else(|e| die(&e.to_string()))
}

fn parse_output_format_inner(args: &[String]) -> Result<OutputFormat, CliArgError> {
    match find_flag_value_inner(args, "--format")? {
        None => Ok(OutputFormat::Human),
        Some(val) => match val.as_str() {
            "human" => Ok(OutputFormat::Human),
            "json" => Ok(OutputFormat::Json),
            other => Err(CliArgError::UnknownOutputFormat {
                got: other.to_string(),
            }),
        },
    }
}

pub(crate) fn parse_compare_mode(args: &[String]) -> CompareMode {
    parse_compare_mode_inner(args).unwrap_or_else(|e| die(&e.to_string()))
}

fn parse_compare_mode_inner(args: &[String]) -> Result<CompareMode, CliArgError> {
    match find_flag_value_inner(args, "--mode")? {
        None => Ok(CompareMode::Memory),
        Some(val) => match val.as_str() {
            "strict" => Ok(CompareMode::Strict),
            "memory" => Ok(CompareMode::Memory),
            "events" => Ok(CompareMode::Events),
            "prefix" => Ok(CompareMode::Prefix),
            other => Err(CliArgError::UnknownCompareMode {
                got: other.to_string(),
            }),
        },
    }
}

/// Find a `--flag <value>` pair in args (two tokens, separated).
///
/// # Errors
///
/// - Flag appears more than once.
/// - Flag is the last token (no following value).
/// - Following token starts with `--` (missing-value upstream).
/// - `--flag=value` form is used.
pub(crate) fn find_flag_value(args: &[String], flag: &str) -> Option<String> {
    find_flag_value_inner(args, flag).unwrap_or_else(|e| die(&e.to_string()))
}

fn find_flag_value_inner(args: &[String], flag: &str) -> Result<Option<String>, CliArgError> {
    let mut found: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a.starts_with(flag) && a.as_bytes().get(flag.len()) == Some(&b'=') {
            return Err(CliArgError::FlagEqNotSupported {
                flag: flag.to_string(),
            });
        }
        if a != flag {
            i += 1;
            continue;
        }
        if found.is_some() {
            return Err(CliArgError::DuplicateFlag {
                flag: flag.to_string(),
            });
        }
        let val = match args.get(i + 1) {
            Some(v) => v.clone(),
            None => {
                return Err(CliArgError::FlagRequiresValue {
                    flag: flag.to_string(),
                });
            }
        };
        if val.starts_with("--") {
            return Err(CliArgError::FlagValueLooksLikeFlag {
                flag: flag.to_string(),
                value: val,
            });
        }
        found = Some(val);
        i += 2;
    }
    Ok(found)
}

/// Parse a `--flag <value>` pair where `value: FromStr`.
pub(crate) fn parse_flag_value<T: std::str::FromStr>(args: &[String], flag: &str) -> Option<T>
where
    T::Err: std::fmt::Display,
{
    find_flag_value(args, flag).map(|v| {
        v.parse()
            .unwrap_or_else(|e| die(&format!("{flag}: cannot parse {v:?}: {e}")))
    })
}

/// Parse a `--flag <hex>` pair as a hex u64 with optional `0x`/`0X` prefix.
pub(crate) fn parse_hex_flag(args: &[String], flag: &str) -> Option<u64> {
    find_flag_value(args, flag).map(|v| parse_hex_u64(&v, flag))
}

/// Parse `s` as a hex u64 with optional `0x`/`0X` prefix.
pub(crate) fn parse_hex_u64(s: &str, context: &str) -> u64 {
    parse_hex_u64_inner(s, context).unwrap_or_else(|e| die(&e.to_string()))
}

fn parse_hex_u64_inner(s: &str, context: &str) -> Result<u64, CliArgError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(CliArgError::EmptyHexValue {
            context: context.to_string(),
        });
    }
    let stripped = strip_hex_prefix(trimmed);
    if stripped.is_empty() {
        return Err(CliArgError::HexPrefixNoDigits {
            context: context.to_string(),
            raw: s.to_string(),
        });
    }
    u64::from_str_radix(stripped, 16).map_err(|source| CliArgError::CannotParseHexU64 {
        context: context.to_string(),
        raw: s.to_string(),
        source,
    })
}

fn parse_hex_u8_inner(s: &str, context: &str) -> Result<u8, CliArgError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(CliArgError::EmptyHexValue {
            context: context.to_string(),
        });
    }
    let stripped = strip_hex_prefix(trimmed);
    if stripped.is_empty() {
        return Err(CliArgError::HexPrefixNoDigits {
            context: context.to_string(),
            raw: s.to_string(),
        });
    }
    if stripped.len() > 2 {
        return Err(CliArgError::HexU8TooLong {
            context: context.to_string(),
            raw: s.to_string(),
            digits: stripped.len(),
        });
    }
    u8::from_str_radix(stripped, 16).map_err(|source| CliArgError::CannotParseHexU8 {
        context: context.to_string(),
        raw: s.to_string(),
        source,
    })
}

pub(crate) fn strip_hex_prefix(s: &str) -> &str {
    s.strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s)
}

/// Parse one `ADDR=VALUE` pair (both hex) from `--patch-byte`.
pub(crate) fn parse_patch_byte_pair(pair: &str) -> (u64, u8) {
    parse_patch_byte_pair_inner(pair).unwrap_or_else(|e| die(&e.to_string()))
}

fn parse_patch_byte_pair_inner(pair: &str) -> Result<(u64, u8), CliArgError> {
    if pair.is_empty() {
        return Err(CliArgError::PatchByteEmpty);
    }
    let mut parts = pair.splitn(2, '=');
    // `splitn(2, _)` on a non-empty `&str` yields >= 1 element; the
    // second element is `None` only when `=` is absent.
    let a_raw = parts
        .next()
        .expect("splitn(2) yields at least one element on a non-empty input");
    let b_raw = parts
        .next()
        .ok_or_else(|| CliArgError::PatchByteMissingEq {
            pair: pair.to_string(),
        })?;
    if a_raw.trim().is_empty() {
        return Err(CliArgError::PatchByteEmptyAddress {
            pair: pair.to_string(),
        });
    }
    if b_raw.trim().is_empty() {
        return Err(CliArgError::PatchByteEmptyValue {
            pair: pair.to_string(),
        });
    }
    if b_raw.contains('=') {
        return Err(CliArgError::PatchByteExtraEq {
            pair: pair.to_string(),
        });
    }
    let addr = parse_hex_u64_inner(a_raw, "--patch-byte address")?;
    let val = parse_hex_u8_inner(b_raw, "--patch-byte value")?;
    Ok((addr, val))
}

/// Flags consumed by `run-game` / `bench-boot` that take a
/// following value argument.
pub(crate) const RUN_GAME_VALUE_FLAGS: &[&str] = &[
    "--title",
    "--content-id",
    "--title-manifest",
    "--vfs-root",
    "--max-steps",
    "--budget",
    "--firmware-dir",
    "--dump-at-pc",
    "--dump-skip",
    "--dump-mem-boot",
    "--dump-mem-fault",
    "--patch-byte",
    "--save-observation",
    "--observation-manifest",
    "--save-boot-summary",
    "--save-state-trace",
    "--checkpoint",
];

/// Locate the positional ELF path in a `run-game` invocation;
/// accepts either ordering of positional path vs. flag pairs.
pub(crate) fn find_run_game_elf_path(args: &[String]) -> Option<String> {
    find_run_game_elf_path_inner(args).unwrap_or_else(|e| die(&e.to_string()))
}

fn find_run_game_elf_path_inner(args: &[String]) -> Result<Option<String>, CliArgError> {
    let mut found: Option<String> = None;
    let mut i = 2; // skip argv[0] and "run-game"
    while i < args.len() {
        let a = &args[i];
        if let Some(known) = RUN_GAME_VALUE_FLAGS
            .iter()
            .find(|f| a.starts_with(*f) && a.as_bytes().get(f.len()) == Some(&b'='))
        {
            return Err(CliArgError::FlagEqNotSupported {
                flag: (*known).to_string(),
            });
        }
        if RUN_GAME_VALUE_FLAGS.contains(&a.as_str()) {
            if args.get(i + 1).is_none() {
                return Err(CliArgError::FlagRequiresValue { flag: a.clone() });
            }
            i += 2;
            continue;
        }
        if a.is_empty() {
            return Err(CliArgError::EmptyPositional);
        }
        if a.starts_with("--") {
            i += 1;
            continue;
        }
        if let Some(existing) = &found {
            return Err(CliArgError::ExtraPositional {
                value: a.clone(),
                existing: existing.clone(),
            });
        }
        found = Some(a.clone());
        i += 1;
    }
    Ok(found)
}

#[cfg(test)]
#[path = "tests/args_tests.rs"]
mod tests;
