//! Command-line argument parsing primitives. Pure over `&[String]`:
//! every function returns a parsed value or calls [`die`] on malformed input.

use cellgov_compare::CompareMode;

use super::exit::die;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Human,
    Json,
}

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
/// Calls [`die`] when the flag appears more than once, or when it is
/// the last token with no following value.
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
        // Skip the value token so it cannot match a later flag
        // comparison if the value coincidentally equals `flag`.
        i += 2;
    }
    found
}

/// Parse a `--flag <value>` pair where `value: FromStr`. Malformed
/// input dies with the flag name and the parse error.
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

/// Parse `s` as a hex u64 with optional `0x`/`0X` prefix. `context`
/// names the flag or field for the error message.
pub(crate) fn parse_hex_u64(s: &str, context: &str) -> u64 {
    let trimmed = s.trim();
    let stripped = strip_hex_prefix(trimmed);
    u64::from_str_radix(stripped, 16)
        .unwrap_or_else(|e| die(&format!("{context}: cannot parse hex {s:?}: {e}")))
}

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

/// Parse one `ADDR=VALUE` pair (both hex) from `--patch-byte`.
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

/// Flags consumed by `run-game` / `bench-boot` that take a following
/// value argument. [`find_run_game_elf_path`] skips past these pairs
/// when locating the positional ELF path.
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

/// Locate the positional ELF path in a `run-game` invocation. Accepts
/// either ordering of positional path vs. flag pairs.
pub(crate) fn find_run_game_elf_path(args: &[String]) -> Option<String> {
    let mut i = 2; // skip argv[0] and "run-game"
    while i < args.len() {
        let a = &args[i];
        if RUN_GAME_VALUE_FLAGS.contains(&a.as_str()) {
            i += 2;
            continue;
        }
        if a.starts_with("--") {
            i += 1;
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
