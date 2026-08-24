//! Boot-command flag parsing -- dump-mem fault ranges and hex CSV lists.

use super::*;

#[test]
fn parse_dump_mem_fault_range_default_len() {
    let (addr, len) = parse_dump_mem_fault_range_inner("0x1000").unwrap();
    assert_eq!(addr, 0x1000);
    assert_eq!(len, DEFAULT_DUMP_LEN);
}

#[test]
fn parse_dump_mem_fault_range_explicit_len() {
    let (addr, len) = parse_dump_mem_fault_range_inner("0x1000:0x100").unwrap();
    assert_eq!(addr, 0x1000);
    assert_eq!(len, 0x100);
}

#[test]
fn parse_dump_mem_fault_range_rejects_zero_len() {
    let err = parse_dump_mem_fault_range_inner("0x1000:0").unwrap_err();
    assert!(err.contains("zero-byte length"), "got: {err}");
}

#[test]
fn parse_dump_mem_fault_range_rejects_overlong_len() {
    let err = parse_dump_mem_fault_range_inner("0x1000:0x100000").unwrap_err();
    assert!(err.contains("exceeds maximum"), "got: {err}");
}

#[test]
fn parse_dump_mem_fault_range_rejects_extra_colon() {
    let err = parse_dump_mem_fault_range_inner("0x1000:0x40:0x80").unwrap_err();
    assert!(err.contains("extra ':'"), "got: {err}");
    assert!(err.contains("0x80"), "got: {err}");
}

#[test]
fn parse_dump_mem_fault_range_rejects_overflow() {
    let err = parse_dump_mem_fault_range_inner("0xffffffffffffffff:0x10").unwrap_err();
    assert!(err.contains("overflows u64"), "got: {err}");
}

#[test]
fn parse_hex_csv_rejects_empty_entry() {
    let err = parse_hex_csv_inner("0x1,,0x2", "--dump-mem-boot").unwrap_err();
    assert!(err.contains("empty entry"), "got: {err}");
    assert!(err.contains("--dump-mem-boot"), "got: {err}");
}

#[test]
fn parse_hex_csv_rejects_trailing_comma() {
    let err = parse_hex_csv_inner("0x1,", "--dump-mem-boot").unwrap_err();
    assert!(err.contains("empty entry"), "got: {err}");
}

#[test]
fn parse_hex_csv_parses_list() {
    let v = parse_hex_csv_inner("0x1,0x2,0x3", "--dump-mem-boot").unwrap();
    assert_eq!(v, vec![1, 2, 3]);
}

#[test]
fn parse_dump_mem_fault_csv_rejects_empty_entry() {
    let err = parse_dump_mem_fault_csv_inner("0x10,,0x20").unwrap_err();
    assert!(err.contains("empty entry"), "got: {err}");
}

#[test]
fn parse_patch_byte_csv_rejects_empty_entry() {
    let err = parse_patch_byte_csv_inner("0x10=0xab,").unwrap_err();
    assert!(err.contains("empty entry"), "got: {err}");
}

fn manifest_with_bench_cap(bench_max_steps: Option<u64>) -> game::manifest::TitleManifest {
    use game::manifest::{CheckpointTrigger, Distribution, GameSource, TitleManifest};
    TitleManifest {
        content_id: "CG_TEST".to_string(),
        short_name: "test".to_string(),
        display_name: "test".to_string(),
        eboot_candidates: vec!["EBOOT.BIN".to_string()],
        year: 2007,
        developer: "test-developer".to_string(),
        engine: "test-engine".to_string(),
        distribution: Distribution::PsnHdd,
        rap_filename: None,
        bench_max_steps,
        checkpoint: CheckpointTrigger::ProcessExit,
        source: GameSource::Hdd,
        rsx_mirror: false,
        rsx_consume: false,
        content: None,
        mounts: Vec::new(),
    }
}

#[test]
fn a_raised_manifest_cap_is_the_bench_default_not_the_hardcoded_one() {
    // The anchor is measured at this cap, and a bench run at any other
    // cap is reported incomparable and gates nothing.
    let title = manifest_with_bench_cap(Some(250_000_000));
    assert_eq!(default_bench_max_steps(&title), 250_000_000);
}

#[test]
fn a_manifest_without_a_bench_cap_takes_the_recorder_default() {
    let title = manifest_with_bench_cap(None);
    assert_eq!(
        default_bench_max_steps(&title) as u64,
        crate::paths::DEFAULT_BENCH_MAX_STEPS
    );
}
