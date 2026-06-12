//! Compare-mode, output-format, and find-flag-value argument parsing.

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
    let err = parse_compare_mode_inner(&args).unwrap_err().to_string();
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
    let err = parse_output_format_inner(&args).unwrap_err().to_string();
    assert!(err.contains("unknown output format: yaml"), "got: {err}");
    assert!(err.contains("human, json"), "got: {err}");
}

#[test]
fn find_flag_value_duplicate_dies() {
    let args = sv(&["cli", "x", "--mode", "strict", "--mode", "memory"]);
    let err = find_flag_value_inner(&args, "--mode")
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("--mode was specified more than once"),
        "got: {err}"
    );
}

#[test]
fn find_flag_value_trailing_flag_dies() {
    let args = sv(&["cli", "x", "--mode"]);
    let err = find_flag_value_inner(&args, "--mode")
        .unwrap_err()
        .to_string();
    assert!(err.contains("--mode requires a value"), "got: {err}");
}

#[test]
fn find_flag_value_rejects_eq_form() {
    let args = sv(&["cli", "x", "--mode=strict"]);
    let err = find_flag_value_inner(&args, "--mode")
        .unwrap_err()
        .to_string();
    assert!(err.contains("--mode=... not supported"), "got: {err}");
    assert!(err.contains("two-token form"), "got: {err}");
}

#[test]
fn find_flag_value_rejects_dash_dash_value() {
    let args = sv(&["cli", "x", "--budget", "--mode", "100"]);
    let err = find_flag_value_inner(&args, "--budget")
        .unwrap_err()
        .to_string();
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
    let err = find_run_game_elf_path_inner(&args).unwrap_err().to_string();
    assert!(err.contains("unexpected extra positional"), "got: {err}");
    assert!(err.contains("garbage"), "got: {err}");
}

#[test]
fn find_run_game_elf_path_rejects_empty_positional() {
    let args = sv(&["cli", "run-game", "", "--title", "flow"]);
    let err = find_run_game_elf_path_inner(&args).unwrap_err().to_string();
    assert!(err.contains("empty positional argument"), "got: {err}");
}

#[test]
fn find_run_game_elf_path_rejects_trailing_value_flag() {
    let args = sv(&["cli", "run-game", "EBOOT.elf", "--budget"]);
    let err = find_run_game_elf_path_inner(&args).unwrap_err().to_string();
    assert!(err.contains("--budget requires a value"), "got: {err}");
}

#[test]
fn find_run_game_elf_path_rejects_eq_form_in_value_flag() {
    let args = sv(&["cli", "run-game", "--title=flow", "EBOOT.elf"]);
    let err = find_run_game_elf_path_inner(&args).unwrap_err().to_string();
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
    let err = parse_patch_byte_pair_inner("").unwrap_err().to_string();
    assert!(err.contains("empty argument"), "got: {err}");
}

#[test]
fn parse_patch_byte_pair_rejects_missing_eq() {
    let err = parse_patch_byte_pair_inner("0x100")
        .unwrap_err()
        .to_string();
    assert!(err.contains("missing '='"), "got: {err}");
}

#[test]
fn parse_patch_byte_pair_rejects_empty_left() {
    let err = parse_patch_byte_pair_inner("=0xab")
        .unwrap_err()
        .to_string();
    assert!(err.contains("empty address"), "got: {err}");
}

#[test]
fn parse_patch_byte_pair_rejects_empty_right() {
    let err = parse_patch_byte_pair_inner("0x100=")
        .unwrap_err()
        .to_string();
    assert!(err.contains("empty value"), "got: {err}");
}

#[test]
fn parse_patch_byte_pair_rejects_extra_eq() {
    let err = parse_patch_byte_pair_inner("0x100=0xab=0xcd")
        .unwrap_err()
        .to_string();
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
    let err = parse_hex_u64_inner("", "ctx").unwrap_err().to_string();
    assert!(err.contains("empty hex value"), "got: {err}");
}

#[test]
fn parse_hex_u64_rejects_bare_prefix() {
    let err = parse_hex_u64_inner("0x", "ctx").unwrap_err().to_string();
    assert!(err.contains("hex prefix with no digits"), "got: {err}");
}

#[test]
fn parse_hex_u64_rejects_non_hex() {
    let err = parse_hex_u64_inner("nothex", "ctx")
        .unwrap_err()
        .to_string();
    assert!(err.contains("cannot parse hex"), "got: {err}");
}

#[test]
fn parse_hex_u64_rejects_negative() {
    let err = parse_hex_u64_inner("-1", "ctx").unwrap_err().to_string();
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
    let err = parse_hex_u8_inner("abcd", "ctx").unwrap_err().to_string();
    assert!(err.contains("expected 1-2 hex digits"), "got: {err}");
    assert!(err.contains("got 4"), "got: {err}");
}

#[test]
fn parse_hex_u8_rejects_empty() {
    let err = parse_hex_u8_inner("", "ctx").unwrap_err().to_string();
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
