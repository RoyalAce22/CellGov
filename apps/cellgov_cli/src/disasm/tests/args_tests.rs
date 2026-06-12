//! Disasm argument parsing -- hex values, vaddr alignment, and count bounds.

use super::*;

fn args_vec(extra: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = ["cellgov_cli", "disasm", "/tmp/elf"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    for s in extra {
        v.push(s.to_string());
    }
    v
}

#[test]
fn parse_hex_accepts_with_and_without_prefix() {
    assert_eq!(parse_hex_u64("0x10"), Some(0x10));
    assert_eq!(parse_hex_u64("0X10"), Some(0x10));
    assert_eq!(parse_hex_u64("10"), Some(0x10));
    assert_eq!(parse_hex_u64("deadbeef"), Some(0xdead_beef));
}

#[test]
fn parse_hex_rejects_garbage() {
    assert_eq!(parse_hex_u64(""), None);
    assert_eq!(parse_hex_u64("0x"), None);
    assert_eq!(parse_hex_u64("0xZZ"), None);
    assert_eq!(parse_hex_u64("ffffffffffffffff0"), None); // overflow
}

#[test]
fn parse_args_requires_vaddr() {
    let err = parse_args(&args_vec(&[])).unwrap_err();
    assert_eq!(err, ArgError::Usage);
}

#[test]
fn parse_args_rejects_unaligned_vaddr() {
    let err = parse_args(&args_vec(&["--vaddr", "0x10002"])).unwrap_err();
    assert_eq!(err, ArgError::UnalignedVaddr(0x10002));
}

#[test]
fn parse_args_rejects_count_zero() {
    let err = parse_args(&args_vec(&["--vaddr", "0x10000", "--count", "0"])).unwrap_err();
    assert_eq!(err, ArgError::CountIsZero);
}

#[test]
fn parse_args_rejects_count_over_max() {
    let err = parse_args(&args_vec(&["--vaddr", "0x10000", "--count", "1000000"])).unwrap_err();
    assert_eq!(err, ArgError::CountTooLarge(1_000_000));
}

#[test]
fn parse_args_reports_missing_value_for_specific_flag() {
    let err = parse_args(&args_vec(&["--vaddr"])).unwrap_err();
    assert_eq!(err, ArgError::MissingValueFor("--vaddr"));
    let err = parse_args(&args_vec(&["--vaddr", "0x10000", "--count"])).unwrap_err();
    assert_eq!(err, ArgError::MissingValueFor("--count"));
}

#[test]
fn parse_args_unknown_flag_is_specific() {
    let err = parse_args(&args_vec(&["--vaddr", "0x10000", "--lol"])).unwrap_err();
    assert_eq!(err, ArgError::UnknownFlag("--lol".to_string()));
}

#[test]
fn parse_args_invalid_hex_includes_value() {
    let err = parse_args(&args_vec(&["--vaddr", "nothex!"])).unwrap_err();
    assert_eq!(
        err,
        ArgError::InvalidHex {
            flag: "--vaddr",
            value: "nothex!".to_string()
        }
    );
}

#[test]
fn parse_args_happy_path() {
    let argv = args_vec(&["--vaddr", "0x10000", "--count", "32"]);
    let p = parse_args(&argv).unwrap();
    assert_eq!(p.vaddr, 0x10000);
    assert_eq!(p.count, 32);
    assert_eq!(p.elf_path, "/tmp/elf");
}

#[test]
fn parse_args_accepts_count_at_max() {
    let argv = args_vec(&["--vaddr", "0x10000", "--count", "65536"]);
    let p = parse_args(&argv).unwrap();
    assert_eq!(p.count, MAX_COUNT);
}
