//! Command-line argument parsing primitives over `&[String]`.
//!
//! Every parser has a public wrapper that calls [`die`] on malformed
//! input and a private `_inner` variant that returns `Result<_, String>`.
//! Error paths are tested against the `_inner` form because [`die`]
//! calls `process::exit`, which would abort the test harness.

use cellgov_compare::CompareMode;

use super::exit::die;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Human,
    Json,
}

pub(crate) fn parse_output_format(args: &[String]) -> OutputFormat {
    parse_output_format_inner(args).unwrap_or_else(|e| die(&e))
}

fn parse_output_format_inner(args: &[String]) -> Result<OutputFormat, String> {
    match find_flag_value_inner(args, "--format")? {
        None => Ok(OutputFormat::Human),
        Some(val) => match val.as_str() {
            "human" => Ok(OutputFormat::Human),
            "json" => Ok(OutputFormat::Json),
            other => Err(format!(
                "unknown output format: {other}\nvalid formats: human, json"
            )),
        },
    }
}

pub(crate) fn parse_compare_mode(args: &[String]) -> CompareMode {
    parse_compare_mode_inner(args).unwrap_or_else(|e| die(&e))
}

fn parse_compare_mode_inner(args: &[String]) -> Result<CompareMode, String> {
    match find_flag_value_inner(args, "--mode")? {
        None => Ok(CompareMode::Memory),
        Some(val) => match val.as_str() {
            "strict" => Ok(CompareMode::Strict),
            "memory" => Ok(CompareMode::Memory),
            "events" => Ok(CompareMode::Events),
            "prefix" => Ok(CompareMode::Prefix),
            other => Err(format!(
                "unknown compare mode: {other}\nvalid modes: strict, memory, events, prefix"
            )),
        },
    }
}

/// Find a `--flag <value>` pair in args. The two-token form is the
/// only accepted shape; `--flag=value` is rejected with a clear error.
///
/// # Errors
///
/// - Flag appears more than once.
/// - Flag is the last token (no following value).
/// - Following token starts with `--` (cross-flag collision: e.g.
///   `--budget --mode 100`; reject because no current CLI value
///   legitimately starts with `--`, so this is almost always a
///   missing-value upstream).
/// - `--flag=value` form is used.
///
/// The cross-flag check is best-effort: a stateless per-call scan
/// cannot fully resolve which token is a value vs. which is a flag
/// across subcommands. A single tokenization pass against a per-
/// subcommand flag spec would close the residue.
pub(crate) fn find_flag_value(args: &[String], flag: &str) -> Option<String> {
    find_flag_value_inner(args, flag).unwrap_or_else(|e| die(&e))
}

fn find_flag_value_inner(args: &[String], flag: &str) -> Result<Option<String>, String> {
    let mut found: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a.starts_with(flag) && a.as_bytes().get(flag.len()) == Some(&b'=') {
            return Err(format!(
                "{flag}=... not supported; use the two-token form: {flag} <value>"
            ));
        }
        if a != flag {
            i += 1;
            continue;
        }
        if found.is_some() {
            return Err(format!(
                "{flag} was specified more than once; pass it exactly once"
            ));
        }
        let val = match args.get(i + 1) {
            Some(v) => v.clone(),
            None => return Err(format!("{flag} requires a value")),
        };
        if val.starts_with("--") {
            return Err(format!(
                "{flag} expects a value but got flag-like token {val:?}; \
                 likely a missing value upstream"
            ));
        }
        found = Some(val);
        // Skip the value token; the same flag name appearing as its
        // own value is not a second occurrence.
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
    parse_hex_u64_inner(s, context).unwrap_or_else(|e| die(&e))
}

fn parse_hex_u64_inner(s: &str, context: &str) -> Result<u64, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(format!("{context}: empty hex value"));
    }
    let stripped = strip_hex_prefix(trimmed);
    if stripped.is_empty() {
        return Err(format!("{context}: hex prefix with no digits in {s:?}"));
    }
    u64::from_str_radix(stripped, 16).map_err(|e| format!("{context}: cannot parse hex {s:?}: {e}"))
}

fn parse_hex_u8_inner(s: &str, context: &str) -> Result<u8, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(format!("{context}: empty hex value"));
    }
    let stripped = strip_hex_prefix(trimmed);
    if stripped.is_empty() {
        return Err(format!("{context}: hex prefix with no digits in {s:?}"));
    }
    // Pre-check keeps the diagnostic in "digits"-space instead of
    // letting `from_str_radix` surface `PosOverflow` on overlong input.
    if stripped.len() > 2 {
        return Err(format!(
            "{context}: expected 1-2 hex digits, got {n} in {s:?}",
            n = stripped.len()
        ));
    }
    u8::from_str_radix(stripped, 16)
        .map_err(|e| format!("{context}: cannot parse hex u8 {s:?}: {e}"))
}

pub(crate) fn strip_hex_prefix(s: &str) -> &str {
    s.strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s)
}

/// Parse one `ADDR=VALUE` pair (both hex) from `--patch-byte`.
pub(crate) fn parse_patch_byte_pair(pair: &str) -> (u64, u8) {
    parse_patch_byte_pair_inner(pair).unwrap_or_else(|e| die(&e))
}

fn parse_patch_byte_pair_inner(pair: &str) -> Result<(u64, u8), String> {
    if pair.is_empty() {
        return Err("--patch-byte: empty argument (expected ADDR=VALUE)".to_string());
    }
    let mut parts = pair.splitn(2, '=');
    // `splitn(2, _)` on a non-empty `&str` yields >= 1 element; the
    // second element is `None` only when `=` is absent.
    let a_raw = parts
        .next()
        .expect("splitn(2) yields at least one element on a non-empty input");
    let b_raw = parts
        .next()
        .ok_or_else(|| format!("--patch-byte: missing '=' in {pair:?} (expected ADDR=VALUE)"))?;
    if a_raw.trim().is_empty() {
        return Err(format!(
            "--patch-byte: empty address in {pair:?} (expected ADDR=VALUE)"
        ));
    }
    if b_raw.trim().is_empty() {
        return Err(format!(
            "--patch-byte: empty value in {pair:?} (expected ADDR=VALUE)"
        ));
    }
    // A third `=` lands inside `b_raw` because `splitn(2, '=')` only
    // splits the first occurrence. Catch it here so the diagnostic
    // names the real problem; otherwise `parse_hex_u8_inner`'s
    // 1-2-digit length check blames the digit count.
    if b_raw.contains('=') {
        return Err(format!(
            "--patch-byte: extra '=' in {pair:?} (expected ADDR=VALUE)"
        ));
    }
    let addr = parse_hex_u64_inner(a_raw, "--patch-byte address")?;
    let val = parse_hex_u8_inner(b_raw, "--patch-byte value")?;
    Ok((addr, val))
}

/// Flags consumed by `run-game` / `bench-boot` that take a following
/// value argument. The `flag_table_invariants` test pins the shape
/// (every entry `--`-prefixed and unique) against developer drift.
pub(crate) const RUN_GAME_VALUE_FLAGS: &[&str] = &[
    "--title",
    "--content-id",
    "--title-manifest",
    "--vfs-root",
    "--max-steps",
    "--budget",
    "--firmware-dir",
    "--boot-mode",
    "--dump-at-pc",
    "--dump-skip",
    "--dump-mem-boot",
    "--dump-mem-fault",
    "--patch-byte",
    "--save-observation",
    "--observation-manifest",
    "--checkpoint",
];

/// Locate the positional ELF path in a `run-game` invocation. Accepts
/// either ordering of positional path vs. flag pairs. Extra
/// positionals, empty positionals, `--flag=value` forms, and trailing
/// value-flags-without-values are all rejected explicitly so the user
/// gets a diagnostic naming the real problem.
pub(crate) fn find_run_game_elf_path(args: &[String]) -> Option<String> {
    find_run_game_elf_path_inner(args).unwrap_or_else(|e| die(&e))
}

fn find_run_game_elf_path_inner(args: &[String]) -> Result<Option<String>, String> {
    let mut found: Option<String> = None;
    let mut i = 2; // skip argv[0] and "run-game"
    while i < args.len() {
        let a = &args[i];
        if let Some(known) = RUN_GAME_VALUE_FLAGS
            .iter()
            .find(|f| a.starts_with(*f) && a.as_bytes().get(f.len()) == Some(&b'='))
        {
            return Err(format!(
                "{known}=... not supported; use the two-token form: {known} <value>"
            ));
        }
        if RUN_GAME_VALUE_FLAGS.contains(&a.as_str()) {
            if args.get(i + 1).is_none() {
                return Err(format!("{a} requires a value"));
            }
            i += 2;
            continue;
        }
        if a.is_empty() {
            return Err("unexpected empty positional argument".to_string());
        }
        if a.starts_with("--") {
            i += 1;
            continue;
        }
        if let Some(existing) = &found {
            return Err(format!(
                "unexpected extra positional: {a:?} (already have {existing:?})"
            ));
        }
        found = Some(a.clone());
        i += 1;
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_compare_mode_defaults_to_memory() {
        let args: Vec<String> = sv(&["cli", "compare", "isa"]);
        assert_eq!(parse_compare_mode(&args), CompareMode::Memory);
    }

    #[test]
    fn parse_compare_mode_reads_flag() {
        let args: Vec<String> = sv(&["cli", "compare", "isa", "--mode", "strict"]);
        assert_eq!(parse_compare_mode(&args), CompareMode::Strict);
    }

    #[test]
    fn parse_compare_mode_unknown_value_errors() {
        let args = sv(&["cli", "compare", "isa", "--mode", "bogus"]);
        let err = parse_compare_mode_inner(&args).unwrap_err();
        assert!(err.contains("unknown compare mode: bogus"), "got: {err}");
        assert!(err.contains("strict, memory, events, prefix"), "got: {err}");
    }

    #[test]
    fn parse_output_format_defaults_to_human() {
        let args: Vec<String> = sv(&["cli", "compare", "isa"]);
        assert_eq!(parse_output_format(&args), OutputFormat::Human);
    }

    #[test]
    fn parse_output_format_reads_json_flag() {
        let args: Vec<String> = sv(&["cli", "compare", "isa", "--format", "json"]);
        assert_eq!(parse_output_format(&args), OutputFormat::Json);
    }

    #[test]
    fn parse_output_format_unknown_value_errors() {
        let args = sv(&["cli", "compare", "isa", "--format", "yaml"]);
        let err = parse_output_format_inner(&args).unwrap_err();
        assert!(err.contains("unknown output format: yaml"), "got: {err}");
        assert!(err.contains("human, json"), "got: {err}");
    }

    #[test]
    fn find_flag_value_duplicate_dies() {
        let args = sv(&["cli", "x", "--mode", "strict", "--mode", "memory"]);
        let err = find_flag_value_inner(&args, "--mode").unwrap_err();
        assert!(
            err.contains("--mode was specified more than once"),
            "got: {err}"
        );
    }

    #[test]
    fn find_flag_value_trailing_flag_dies() {
        let args = sv(&["cli", "x", "--mode"]);
        let err = find_flag_value_inner(&args, "--mode").unwrap_err();
        assert!(err.contains("--mode requires a value"), "got: {err}");
    }

    #[test]
    fn find_flag_value_rejects_eq_form() {
        let args = sv(&["cli", "x", "--mode=strict"]);
        let err = find_flag_value_inner(&args, "--mode").unwrap_err();
        assert!(err.contains("--mode=... not supported"), "got: {err}");
        assert!(err.contains("two-token form"), "got: {err}");
    }

    #[test]
    fn find_flag_value_rejects_dash_dash_value() {
        let args = sv(&["cli", "x", "--budget", "--mode", "100"]);
        let err = find_flag_value_inner(&args, "--budget").unwrap_err();
        assert!(err.contains("--budget expects a value"), "got: {err}");
        assert!(err.contains("flag-like token"), "got: {err}");
    }

    #[test]
    fn find_run_game_elf_path_accepts_elf_first() {
        let args = sv(&["cli", "run-game", "EBOOT.elf", "--title", "flow"]);
        assert_eq!(find_run_game_elf_path(&args), Some("EBOOT.elf".to_string()));
    }

    #[test]
    fn find_run_game_elf_path_accepts_title_first() {
        let args = sv(&["cli", "run-game", "--title", "sshd", "EBOOT.BIN"]);
        assert_eq!(find_run_game_elf_path(&args), Some("EBOOT.BIN".to_string()));
    }

    #[test]
    fn find_run_game_elf_path_skips_boolean_flags() {
        let args = sv(&[
            "cli",
            "run-game",
            "--trace",
            "--profile",
            "--title",
            "flow",
            "game.elf",
        ]);
        assert_eq!(find_run_game_elf_path(&args), Some("game.elf".to_string()));
    }

    #[test]
    fn find_run_game_elf_path_returns_none_when_missing() {
        let args = sv(&["cli", "run-game", "--title", "flow"]);
        assert_eq!(find_run_game_elf_path(&args), None);
    }

    #[test]
    fn find_run_game_elf_path_skips_value_flag_that_looks_like_path() {
        let args = sv(&[
            "cli",
            "run-game",
            "--firmware-dir",
            "/path/to/fw",
            "--title",
            "flow",
            "EBOOT.elf",
        ]);
        assert_eq!(find_run_game_elf_path(&args), Some("EBOOT.elf".to_string()));
    }

    #[test]
    fn find_run_game_elf_path_skips_numeric_value_flags() {
        let args = sv(&[
            "cli",
            "run-game",
            "--budget",
            "100",
            "--max-steps",
            "500000",
            "--title",
            "sshd",
            "EBOOT.elf",
        ]);
        assert_eq!(find_run_game_elf_path(&args), Some("EBOOT.elf".to_string()));
    }

    #[test]
    fn find_run_game_elf_path_rejects_extra_positional() {
        let args = sv(&["cli", "run-game", "EBOOT.elf", "garbage", "--title", "flow"]);
        let err = find_run_game_elf_path_inner(&args).unwrap_err();
        assert!(err.contains("unexpected extra positional"), "got: {err}");
        assert!(err.contains("garbage"), "got: {err}");
    }

    #[test]
    fn find_run_game_elf_path_rejects_empty_positional() {
        let args = sv(&["cli", "run-game", "", "--title", "flow"]);
        let err = find_run_game_elf_path_inner(&args).unwrap_err();
        assert!(err.contains("empty positional argument"), "got: {err}");
    }

    #[test]
    fn find_run_game_elf_path_rejects_trailing_value_flag() {
        let args = sv(&["cli", "run-game", "EBOOT.elf", "--budget"]);
        let err = find_run_game_elf_path_inner(&args).unwrap_err();
        assert!(err.contains("--budget requires a value"), "got: {err}");
    }

    #[test]
    fn find_run_game_elf_path_rejects_eq_form_in_value_flag() {
        let args = sv(&["cli", "run-game", "--title=flow", "EBOOT.elf"]);
        let err = find_run_game_elf_path_inner(&args).unwrap_err();
        assert!(err.contains("--title=... not supported"), "got: {err}");
    }

    #[test]
    fn parse_patch_byte_pair_accepts_hex_with_prefix() {
        assert_eq!(parse_patch_byte_pair("0x1000=0xab"), (0x1000, 0xab));
    }

    #[test]
    fn parse_patch_byte_pair_accepts_bare_hex() {
        assert_eq!(parse_patch_byte_pair("1000=ab"), (0x1000, 0xab));
    }

    #[test]
    fn parse_patch_byte_pair_tolerates_surrounding_whitespace() {
        assert_eq!(parse_patch_byte_pair(" 0x20 = 0xff "), (0x20, 0xff));
    }

    #[test]
    fn parse_patch_byte_pair_rejects_empty() {
        let err = parse_patch_byte_pair_inner("").unwrap_err();
        assert!(err.contains("empty argument"), "got: {err}");
    }

    #[test]
    fn parse_patch_byte_pair_rejects_missing_eq() {
        let err = parse_patch_byte_pair_inner("0x100").unwrap_err();
        assert!(err.contains("missing '='"), "got: {err}");
    }

    #[test]
    fn parse_patch_byte_pair_rejects_empty_left() {
        let err = parse_patch_byte_pair_inner("=0xab").unwrap_err();
        assert!(err.contains("empty address"), "got: {err}");
    }

    #[test]
    fn parse_patch_byte_pair_rejects_empty_right() {
        let err = parse_patch_byte_pair_inner("0x100=").unwrap_err();
        assert!(err.contains("empty value"), "got: {err}");
    }

    #[test]
    fn parse_patch_byte_pair_rejects_extra_eq() {
        let err = parse_patch_byte_pair_inner("0x100=0xab=0xcd").unwrap_err();
        assert!(err.contains("extra '='"), "got: {err}");
        assert!(err.contains("0x100=0xab=0xcd"), "got: {err}");
    }

    #[test]
    fn parse_hex_u64_accepts_lower_prefix() {
        assert_eq!(parse_hex_u64("0xdeadbeef", "ctx"), 0xdeadbeef);
    }

    #[test]
    fn parse_hex_u64_accepts_upper_prefix() {
        assert_eq!(parse_hex_u64("0XDEADBEEF", "ctx"), 0xdeadbeef);
    }

    #[test]
    fn parse_hex_u64_accepts_bare_hex() {
        assert_eq!(parse_hex_u64("ff", "ctx"), 0xff);
    }

    #[test]
    fn parse_hex_u64_rejects_empty() {
        let err = parse_hex_u64_inner("", "ctx").unwrap_err();
        assert!(err.contains("empty hex value"), "got: {err}");
    }

    #[test]
    fn parse_hex_u64_rejects_bare_prefix() {
        let err = parse_hex_u64_inner("0x", "ctx").unwrap_err();
        assert!(err.contains("hex prefix with no digits"), "got: {err}");
    }

    #[test]
    fn parse_hex_u64_rejects_non_hex() {
        let err = parse_hex_u64_inner("nothex", "ctx").unwrap_err();
        assert!(err.contains("cannot parse hex"), "got: {err}");
    }

    #[test]
    fn parse_hex_u64_rejects_negative() {
        let err = parse_hex_u64_inner("-1", "ctx").unwrap_err();
        assert!(err.contains("cannot parse hex"), "got: {err}");
    }

    #[test]
    fn parse_hex_u8_accepts_two_digits() {
        assert_eq!(parse_hex_u8_inner("ab", "ctx").unwrap(), 0xab);
    }

    #[test]
    fn parse_hex_u8_accepts_one_digit() {
        assert_eq!(parse_hex_u8_inner("0x5", "ctx").unwrap(), 0x05);
    }

    #[test]
    fn parse_hex_u8_rejects_overlong_with_clear_message() {
        let err = parse_hex_u8_inner("abcd", "ctx").unwrap_err();
        assert!(err.contains("expected 1-2 hex digits"), "got: {err}");
        assert!(err.contains("got 4"), "got: {err}");
    }

    #[test]
    fn parse_hex_u8_rejects_empty() {
        let err = parse_hex_u8_inner("", "ctx").unwrap_err();
        assert!(err.contains("empty hex value"), "got: {err}");
    }

    #[test]
    fn strip_hex_prefix_handles_both_cases() {
        assert_eq!(strip_hex_prefix("0xab"), "ab");
        assert_eq!(strip_hex_prefix("0XAB"), "AB");
        assert_eq!(strip_hex_prefix("ab"), "ab");
        assert_eq!(strip_hex_prefix(""), "");
        assert_eq!(strip_hex_prefix("0x"), "");
    }

    #[test]
    fn parse_flag_value_parses_usize() {
        let args = sv(&["cli", "x", "--max-steps", "42"]);
        let v: Option<usize> = parse_flag_value(&args, "--max-steps");
        assert_eq!(v, Some(42));
    }

    #[test]
    fn parse_flag_value_absent_returns_none() {
        let args = sv(&["cli", "x"]);
        let v: Option<usize> = parse_flag_value(&args, "--max-steps");
        assert_eq!(v, None);
    }

    #[test]
    fn parse_hex_flag_reads_value() {
        let args = sv(&["cli", "x", "--dump-at-pc", "0x10000"]);
        assert_eq!(parse_hex_flag(&args, "--dump-at-pc"), Some(0x10000));
    }

    #[test]
    fn parse_hex_flag_absent_returns_none() {
        let args = sv(&["cli", "x"]);
        assert_eq!(parse_hex_flag(&args, "--dump-at-pc"), None);
    }

    #[test]
    fn flag_table_invariants() {
        let mut seen = std::collections::BTreeSet::new();
        for flag in RUN_GAME_VALUE_FLAGS {
            assert!(flag.starts_with("--"), "flag missing -- prefix: {flag}");
            assert!(flag.len() > 2, "flag has empty name: {flag}");
            assert!(seen.insert(*flag), "duplicate flag in table: {flag}");
        }
    }
}
