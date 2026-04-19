//! Command-line argument parsing primitives.
//!
//! Pure over `&[String]` input: every function here either returns
//! a parsed value or calls [`die`] on malformed input. No IO, no
//! scenario knowledge, no subcommand logic -- those live in the
//! per-command modules under `cli/`. The single dependency is
//! [`crate::cli::exit::die`], the leaf error channel used across
//! the CLI.

use cellgov_compare::CompareMode;

use super::exit::die;

/// Output-format selector for commands that support both
/// human-readable and JSON output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Human,
    Json,
}

/// Parse `--format human|json` from CLI args. Defaults to `Human`.
pub(crate) fn parse_output_format(args: &[String]) -> OutputFormat {
    match find_flag_value(args, "--format") {
        None => OutputFormat::Human,
        Some(val) => match val.as_str() {
            "human" => OutputFormat::Human,
            "json" => OutputFormat::Json,
            other => die(&format!(
                "unknown output format: {other}\nvalid formats: human, json"
            )),
        },
    }
}

/// Parse `--mode <mode>` from CLI args. Defaults to `Memory`.
pub(crate) fn parse_compare_mode(args: &[String]) -> CompareMode {
    match find_flag_value(args, "--mode") {
        None => CompareMode::Memory,
        Some(val) => match val.as_str() {
            "strict" => CompareMode::Strict,
            "memory" => CompareMode::Memory,
            "events" => CompareMode::Events,
            "prefix" => CompareMode::Prefix,
            other => die(&format!(
                "unknown compare mode: {other}\nvalid modes: strict, memory, events, prefix"
            )),
        },
    }
}

/// Find a `--flag <value>` pair in args.
///
/// Returns:
/// - `None` if the flag does not appear at all.
/// - `Some(value)` if the flag appears exactly once and is followed
///   by a value token.
/// - Calls [`die`] when the flag appears more than once (the previous
///   first-wins behavior silently dropped subsequent flags; scripts
///   that append flags produced non-obvious results).
/// - Calls [`die`] when the flag is the last token (trailing flag
///   with no value silently became "flag not present").
pub(crate) fn find_flag_value(args: &[String], flag: &str) -> Option<String> {
    let mut found: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] != flag {
            i += 1;
            continue;
        }
        if found.is_some() {
            die(&format!(
                "{flag} was specified more than once; pass it exactly once"
            ));
        }
        let val = args
            .get(i + 1)
            .cloned()
            .unwrap_or_else(|| die(&format!("{flag} requires a value")));
        found = Some(val);
        // Skip the value token so it cannot match a later `flag`
        // comparison if the value coincidentally equals `flag`
        // (pathological but possible with arbitrary strings).
        i += 2;
    }
    found
}

/// Parse a `--flag <value>` pair where `value` is a `FromStr`
/// instance. Malformed input dies with a specific message instead
/// of silently falling back to the caller-supplied default -- the
/// old `.and_then(|v| v.parse().ok()).unwrap_or(default)` pattern
/// hid typos in long-running benches.
///
/// Intended T set: unsigned integer types (usize, u32, u64). The
/// trait bound only requires `T::Err: Display`, so odd T::Err
/// types would not produce helpful errors; stick with integer
/// types until a concrete need arises.
pub(crate) fn parse_flag_value<T: std::str::FromStr>(args: &[String], flag: &str) -> Option<T>
where
    T::Err: std::fmt::Display,
{
    find_flag_value(args, flag).map(|v| {
        v.parse()
            .unwrap_or_else(|e| die(&format!("{flag}: cannot parse {v:?}: {e}")))
    })
}

/// Parse a `--flag <hex>` pair where `value` is interpreted as a
/// hex u64 (with optional `0x`/`0X` prefix). Malformed input dies.
pub(crate) fn parse_hex_flag(args: &[String], flag: &str) -> Option<u64> {
    find_flag_value(args, flag).map(|v| parse_hex_u64(&v, flag))
}

/// Parse `s` as a hex u64 with optional `0x`/`0X` prefix.
/// `context` names the flag / field for the error message so the
/// hex-parsing sites (--dump-at-pc, --dump-mem entries,
/// --patch-byte halves) route through one helper and emit a
/// consistent error shape.
pub(crate) fn parse_hex_u64(s: &str, context: &str) -> u64 {
    let trimmed = s.trim();
    let stripped = strip_hex_prefix(trimmed);
    u64::from_str_radix(stripped, 16)
        .unwrap_or_else(|e| die(&format!("{context}: cannot parse hex {s:?}: {e}")))
}

/// Parse `s` as a hex u8 with optional `0x`/`0X` prefix. Used by
/// `--patch-byte` where the value half must fit in a single byte.
pub(crate) fn parse_hex_u8(s: &str, context: &str) -> u8 {
    let trimmed = s.trim();
    let stripped = strip_hex_prefix(trimmed);
    u8::from_str_radix(stripped, 16)
        .unwrap_or_else(|e| die(&format!("{context}: cannot parse hex u8 {s:?}: {e}")))
}

pub(crate) fn strip_hex_prefix(s: &str) -> &str {
    s.strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s)
}

/// Parse one `ADDR=VALUE` pair from the `--patch-byte` CLI flag.
/// Both halves are hex (with optional `0x` prefix). Dies with a
/// specific message when the `=` is missing or either half fails
/// to parse.
pub(crate) fn parse_patch_byte_pair(pair: &str) -> (u64, u8) {
    let mut parts = pair.splitn(2, '=');
    let a_raw = parts.next().unwrap_or("");
    let b_raw = parts.next().unwrap_or_else(|| {
        die(&format!(
            "--patch-byte: missing '=' in {pair:?} (expected ADDR=VALUE)"
        ))
    });
    let addr = parse_hex_u64(a_raw, "--patch-byte address");
    let val = parse_hex_u8(b_raw, "--patch-byte value");
    (addr, val)
}

/// Flags that `run-game` / `bench-boot` accept which take a
/// following value argument. Used by [`find_run_game_elf_path`] to
/// skip over `--flag VALUE` pairs when locating the positional
/// ELF path.
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
    "--dump-mem",
    "--patch-byte",
    "--save-observation",
    "--observation-manifest",
    "--checkpoint",
];

/// Locate the positional ELF path in a `run-game` invocation,
/// skipping over recognized `--flag VALUE` pairs and standalone
/// `--flag` toggles. Accepts both orderings -- `run-game <elf>
/// --title flow` and `run-game --title flow <elf>` -- without
/// caring which came first.
pub(crate) fn find_run_game_elf_path(args: &[String]) -> Option<String> {
    let mut i = 2; // skip argv[0] and "run-game"
    while i < args.len() {
        let a = &args[i];
        if RUN_GAME_VALUE_FLAGS.contains(&a.as_str()) {
            i += 2; // skip flag and its value
            continue;
        }
        if a.starts_with("--") {
            i += 1; // boolean flag
            continue;
        }
        return Some(a.clone());
    }
    None
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
}
