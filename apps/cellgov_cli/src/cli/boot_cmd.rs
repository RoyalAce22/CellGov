//! Dispatch for the boot-family subcommands: `run-game`,
//! `bench-boot-once`, and `bench-boot`. The guest-side pipeline
//! lives in the sibling `game` module.

use cellgov_compare::BootOutcome;
use cellgov_time::Budget;

use crate::game;
use crate::game::BootMode;

use super::args::{
    find_flag_value, find_run_game_elf_path, parse_flag_value, parse_hex_flag, parse_hex_u64,
    parse_patch_byte_pair,
};
use super::exit::die;
use super::title::{resolve_checkpoint_override, resolve_ps3_vfs_root, resolve_title_manifest};

/// Maximum allowed wall-time disagreement between the two bench-boot
/// subprocess runs, as a percentage of the faster run.
const AGREEMENT_GATE_PERCENT: f64 = 5.0;

/// Where `cellgov_firmware install` lands the minimum viable PRX
/// set's SPRXes by default.
const DEFAULT_FIRMWARE_DIR: &str = "firmware/sys/external";

/// Set by synthetic harnesses (e.g. ps3autotests) to suppress the
/// auto-default.
const DISABLE_DEFAULT_ENV: &str = "CELLGOV_NO_FIRMWARE_DIR";

/// Exit code: two bench-boot runs disagreed on step count or
/// outcome.
const EXIT_DETERMINISM_BREAK: i32 = 3;

/// Exit code: wall-time disagreement exceeded the gate or was
/// unmeasurable.
const EXIT_WALL_DRIFT: i32 = 2;

/// Exit code: a bench-boot subprocess failed or its `BENCH_RESULT`
/// line was unparseable.
const EXIT_SUBPROCESS_FAIL: i32 = 4;

/// `run-game` terminated with a guest fault.
const EXIT_RUN_GAME_FAULT: i32 = 10;
/// `run-game` reached `--max-steps` without hitting the configured
/// checkpoint.
const EXIT_RUN_GAME_MAX_STEPS: i32 = 11;
/// `run-game` exhausted simulated time before reaching a terminal
/// state.
const EXIT_RUN_GAME_TIME_OVERFLOW: i32 = 12;
/// `run-game` completed but the loop logged an anomaly that violates
/// the determinism contract (lost syscall-wake responses).
const EXIT_RUN_GAME_CRITICAL_ANOMALY: i32 = 13;
/// `--save-observation` was supplied but writing the JSON failed.
const EXIT_RUN_GAME_SAVE_OBSERVATION: i32 = 14;

/// Parse `--boot-mode <single-prx|firmware-set>`; defaults to
/// [`BootMode::FirmwareSet`]. See [`cross_check_boot_mode_inner`]
/// for the `firmware-set + no firmware-dir` rejection invariant.
fn resolve_boot_mode(args: &[String]) -> BootMode {
    parse_boot_mode_inner(args).unwrap_or_else(|e| die(&e))
}

fn parse_boot_mode_inner(args: &[String]) -> Result<BootMode, String> {
    match find_flag_value(args, "--boot-mode") {
        None => Ok(BootMode::FirmwareSet),
        Some(v) => __test_parse_boot_mode(&v),
    }
}

/// Parse a single `--boot-mode` value, wrapping the strum error in
/// the CLI's user-facing message.
#[doc(hidden)]
pub(crate) fn __test_parse_boot_mode(v: &str) -> Result<BootMode, String> {
    use std::str::FromStr;
    BootMode::from_str(v).map_err(|_| {
        format!("unknown --boot-mode value: {v:?}\nvalid values: single-prx, firmware-set")
    })
}

/// Reject `--boot-mode firmware-set` with no firmware-dir
/// resolved.
fn cross_check_boot_mode_inner(mode: BootMode, firmware_dir: Option<&str>) -> Result<(), String> {
    match (mode, firmware_dir) {
        (BootMode::FirmwareSet, None) => {
            Err("--boot-mode firmware-set requires a firmware directory; \
             pass --firmware-dir or place SPRXes under firmware/sys/external"
                .to_string())
        }
        _ => Ok(()),
    }
}

/// Explicit `--firmware-dir` wins (validated as an existing
/// directory); otherwise auto-default to [`DEFAULT_FIRMWARE_DIR`]
/// when it exists; `None` falls back to pure HLE.
fn resolve_firmware_dir(args: &[String]) -> Option<String> {
    if let Some(explicit) = find_flag_value(args, "--firmware-dir") {
        if !std::path::Path::new(&explicit).is_dir() {
            die(&format!(
                "--firmware-dir: {explicit:?} is not an existing directory"
            ));
        }
        return Some(explicit);
    }
    if std::env::var_os(DISABLE_DEFAULT_ENV).is_some() {
        return None;
    }
    if std::path::Path::new(DEFAULT_FIRMWARE_DIR).is_dir() {
        eprintln!("boot: --firmware-dir defaulted to {DEFAULT_FIRMWARE_DIR}");
        return Some(DEFAULT_FIRMWARE_DIR.to_string());
    }
    None
}

struct BootInputs {
    title: game::manifest::TitleManifest,
    elf_path: String,
}

/// Resolve the title manifest plus the ELF path the boot will run. A
/// positional ELF override is honoured only when `allow_explicit_elf`
/// is set; bench subcommands pass `false`.
fn resolve_boot_inputs(args: &[String], subcmd: &str, allow_explicit_elf: bool) -> BootInputs {
    let title = resolve_title_manifest(args, subcmd);
    let vfs_root = resolve_ps3_vfs_root(args);
    let explicit = find_run_game_elf_path(args);
    if !allow_explicit_elf {
        if let Some(p) = explicit.as_ref() {
            die(&format!(
                "{subcmd} is title-driven; positional ELF path {p:?} is not accepted here \
                 (use `run-game` for explicit ELF paths)"
            ));
        }
    }
    let elf_path = match explicit {
        Some(p) => p,
        None => match title.resolve_eboot(&vfs_root) {
            Ok(p) => p.to_str().map(|s| s.replace('\\', "/")).unwrap_or_else(|| {
                die(&format!(
                    "{subcmd}: resolved EBOOT path is not valid UTF-8: {}",
                    p.display()
                ))
            }),
            Err(e) => die(&format!("{subcmd}: {e}")),
        },
    };
    BootInputs { title, elf_path }
}

pub(crate) fn run_game(args: &[String]) {
    let inputs = resolve_boot_inputs(args, "run-game", true);
    let max_steps: usize = parse_flag_value(args, "--max-steps").unwrap_or(100_000);
    let trace = args.iter().any(|a| a == "--trace");
    let profile = args.iter().any(|a| a == "--profile");
    let firmware_dir = resolve_firmware_dir(args);
    let boot_mode = resolve_boot_mode(args);
    cross_check_boot_mode_inner(boot_mode, firmware_dir.as_deref()).unwrap_or_else(|e| die(&e));
    let dump_at_pc = parse_hex_flag(args, "--dump-at-pc");
    let dump_skip: u32 = parse_flag_value(args, "--dump-skip").unwrap_or(0);
    if dump_skip > 0 && dump_at_pc.is_none() {
        die("--dump-skip is meaningless without --dump-at-pc");
    }
    let dump_mem_boot_addrs: Vec<u64> = find_flag_value(args, "--dump-mem-boot")
        .map(|v| parse_hex_csv(&v, "--dump-mem-boot"))
        .unwrap_or_default();
    let dump_mem_fault_ranges: Vec<(u64, u64)> = find_flag_value(args, "--dump-mem-fault")
        .map(|v| parse_dump_mem_fault_csv(&v))
        .unwrap_or_default();
    let patch_bytes: Vec<(u64, u8)> = find_flag_value(args, "--patch-byte")
        .map(|v| parse_patch_byte_csv(&v))
        .unwrap_or_default();
    let save_observation = find_flag_value(args, "--save-observation");
    let observation_manifest = find_flag_value(args, "--observation-manifest");
    let save_boot_summary = find_flag_value(args, "--save-boot-summary");
    let save_state_trace = find_flag_value(args, "--save-state-trace");
    let strict_reserved = args.iter().any(|a| a == "--strict-reserved");
    let profile_pairs = args.iter().any(|a| a == "--profile-pairs");
    let budget_override: Option<Budget> =
        parse_flag_value::<u64>(args, "--budget").map(Budget::new);
    let result = game::run_game(game::RunGameOptions {
        title: &inputs.title,
        elf_path: &inputs.elf_path,
        max_steps,
        trace,
        profile,
        firmware_dir: firmware_dir.as_deref(),
        boot_mode,
        dump_at_pc,
        dump_skip,
        patch_bytes: &patch_bytes,
        dump_mem_boot_addrs: &dump_mem_boot_addrs,
        dump_mem_fault_ranges: &dump_mem_fault_ranges,
        save_observation: save_observation.as_deref(),
        observation_manifest: observation_manifest.as_deref(),
        save_boot_summary: save_boot_summary.as_deref(),
        save_state_trace: save_state_trace.as_deref(),
        strict_reserved,
        profile_pairs,
        budget_override,
    });
    let summary = match result {
        Ok(s) => s,
        Err(e) => {
            eprintln!("run-game: {e}");
            std::process::exit(EXIT_RUN_GAME_SAVE_OBSERVATION);
        }
    };
    let code = classify_run_game_exit(&summary);
    if code != 0 {
        std::process::exit(code);
    }
}

/// Map a [`game::RunSummary`] to a process exit code. A critical
/// anomaly (lost syscall-wake response) overrides a clean outcome so
/// determinism-contract violations cannot exit 0.
fn classify_run_game_exit(summary: &game::RunSummary) -> i32 {
    let outcome_code = match summary.outcome {
        BootOutcome::ProcessExit | BootOutcome::RsxWriteCheckpoint | BootOutcome::PcReached(_) => 0,
        BootOutcome::Fault => EXIT_RUN_GAME_FAULT,
        BootOutcome::MaxSteps => EXIT_RUN_GAME_MAX_STEPS,
        BootOutcome::TimeOverflow => EXIT_RUN_GAME_TIME_OVERFLOW,
    };
    if outcome_code == 0 && summary.had_critical_anomaly {
        return EXIT_RUN_GAME_CRITICAL_ANOMALY;
    }
    outcome_code
}

/// Sanity cap, in bytes, on a single `--dump-mem-fault` range.
const MAX_DUMP_LEN: u64 = 64 * 1024;

/// Default LEN when `--dump-mem-fault` is given only an address.
const DEFAULT_DUMP_LEN: u64 = 0x40;

/// Parse `0xADDR` (default LEN) or `0xADDR:LEN`. Both fields parse as
/// hex. LEN must be in `1..=MAX_DUMP_LEN`; ADDR + LEN must not
/// overflow `u64`. Extra `:` segments are rejected.
fn parse_dump_mem_fault_range_inner(spec: &str) -> Result<(u64, u64), String> {
    let mut parts = spec.splitn(3, ':');
    let addr_str = parts.next().unwrap_or("");
    let len_str = parts.next();
    if let Some(rest) = parts.next() {
        return Err(format!(
            "--dump-mem-fault: extra ':' segment {rest:?} in {spec:?} (expected ADDR[:LEN])"
        ));
    }
    let addr = super::args::parse_hex_u64(addr_str, "--dump-mem-fault address");
    let len = match len_str {
        Some(l) => super::args::parse_hex_u64(l, "--dump-mem-fault length"),
        None => DEFAULT_DUMP_LEN,
    };
    if len == 0 {
        return Err(format!(
            "--dump-mem-fault: zero-byte length in {spec:?} (LEN must be > 0)"
        ));
    }
    if len > MAX_DUMP_LEN {
        return Err(format!(
            "--dump-mem-fault: LEN 0x{len:x} exceeds maximum 0x{MAX_DUMP_LEN:x} in {spec:?}"
        ));
    }
    if addr.checked_add(len - 1).is_none() {
        return Err(format!(
            "--dump-mem-fault: ADDR 0x{addr:x} + LEN 0x{len:x} overflows u64 in {spec:?}"
        ));
    }
    Ok((addr, len))
}

/// Parse a comma-separated list of hex addresses; empty entries
/// are rejected.
fn parse_hex_csv(value: &str, flag: &str) -> Vec<u64> {
    parse_hex_csv_inner(value, flag).unwrap_or_else(|e| die(&e))
}

fn parse_hex_csv_inner(value: &str, flag: &str) -> Result<Vec<u64>, String> {
    let mut out = Vec::new();
    for entry in value.split(',') {
        if entry.is_empty() {
            return Err(format!(
                "{flag}: empty entry (leading/trailing/duplicate comma) in {value:?}"
            ));
        }
        out.push(parse_hex_u64(entry, flag));
    }
    Ok(out)
}

/// Parse a comma-separated list of `ADDR[:LEN]` fault-range specs.
fn parse_dump_mem_fault_csv(value: &str) -> Vec<(u64, u64)> {
    parse_dump_mem_fault_csv_inner(value).unwrap_or_else(|e| die(&e))
}

fn parse_dump_mem_fault_csv_inner(value: &str) -> Result<Vec<(u64, u64)>, String> {
    let mut out = Vec::new();
    for entry in value.split(',') {
        if entry.is_empty() {
            return Err(format!(
                "--dump-mem-fault: empty entry (leading/trailing/duplicate comma) in {value:?}"
            ));
        }
        out.push(parse_dump_mem_fault_range_inner(entry)?);
    }
    Ok(out)
}

/// Parse a comma-separated list of `ADDR=VALUE` patch-byte pairs.
fn parse_patch_byte_csv(value: &str) -> Vec<(u64, u8)> {
    parse_patch_byte_csv_inner(value).unwrap_or_else(|e| die(&e))
}

fn parse_patch_byte_csv_inner(value: &str) -> Result<Vec<(u64, u8)>, String> {
    let mut out = Vec::new();
    for entry in value.split(',') {
        if entry.is_empty() {
            return Err(format!(
                "--patch-byte: empty entry (leading/trailing/duplicate comma) in {value:?}"
            ));
        }
        out.push(parse_patch_byte_pair(entry));
    }
    Ok(out)
}

pub(crate) fn bench_boot_once(args: &[String]) {
    let inputs = resolve_boot_inputs(args, "bench-boot-once", false);
    let max_steps: usize = parse_flag_value(args, "--max-steps").unwrap_or(100_000_000);
    let firmware_dir = resolve_firmware_dir(args);
    let boot_mode = resolve_boot_mode(args);
    cross_check_boot_mode_inner(boot_mode, firmware_dir.as_deref()).unwrap_or_else(|e| die(&e));
    let strict_reserved = args.iter().any(|a| a == "--strict-reserved");
    let checkpoint_override = resolve_checkpoint_override(args, "bench-boot-once");
    let budget_override: Option<Budget> =
        parse_flag_value::<u64>(args, "--budget").map(Budget::new);
    game::bench_boot_one_run(game::BenchOptions {
        title: &inputs.title,
        elf_path: &inputs.elf_path,
        max_steps,
        firmware_dir: firmware_dir.as_deref(),
        boot_mode,
        strict_reserved,
        checkpoint_override,
        budget_override,
    });
}

pub(crate) fn bench_boot(args: &[String]) {
    let inputs = resolve_boot_inputs(args, "bench-boot", false);
    let max_steps: usize = parse_flag_value(args, "--max-steps").unwrap_or(100_000_000);
    let firmware_dir = resolve_firmware_dir(args);
    let boot_mode = resolve_boot_mode(args);
    cross_check_boot_mode_inner(boot_mode, firmware_dir.as_deref()).unwrap_or_else(|e| die(&e));
    let strict_reserved = args.iter().any(|a| a == "--strict-reserved");
    let checkpoint_override = resolve_checkpoint_override(args, "bench-boot");
    let budget_override: Option<Budget> =
        parse_flag_value::<u64>(args, "--budget").map(Budget::new);
    let outcome = match game::bench_boot_pair(game::BenchOptions {
        title: &inputs.title,
        elf_path: &inputs.elf_path,
        max_steps,
        firmware_dir: firmware_dir.as_deref(),
        boot_mode,
        strict_reserved,
        checkpoint_override,
        budget_override,
    }) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("bench-boot: {e}");
            let captured_stdout = e.captured_stdout();
            if !captured_stdout.is_empty() {
                eprintln!("stdout:\n{captured_stdout}");
            }
            let captured_stderr = e.captured_stderr();
            if !captured_stderr.is_empty() {
                eprintln!("stderr:\n{captured_stderr}");
            }
            std::process::exit(EXIT_SUBPROCESS_FAIL);
        }
    };
    match outcome.gate {
        game::BenchGate::Pass => {}
        game::BenchGate::DeterminismBreak => {
            eprintln!(
                "bench-boot: determinism break: run 1 steps={} outcome={}, \
                 run 2 steps={} outcome={}; exiting with status {EXIT_DETERMINISM_BREAK}",
                outcome.run1.steps, outcome.run1.outcome, outcome.run2.steps, outcome.run2.outcome,
            );
            std::process::exit(EXIT_DETERMINISM_BREAK);
        }
        game::BenchGate::WallUnmeasurable => {
            eprintln!(
                "bench-boot: wall measurement unusable (zero / non-finite); \
                 run 1 wall {:?}, run 2 wall {:?}; exiting with status {EXIT_WALL_DRIFT}",
                outcome.run1.wall, outcome.run2.wall
            );
            std::process::exit(EXIT_WALL_DRIFT);
        }
        game::BenchGate::WallDriftExceeded => {
            let drift = outcome.drift_pct.unwrap_or(f64::NAN);
            eprintln!(
                "bench-boot: wall disagreement {drift:.2}% exceeds {AGREEMENT_GATE_PERCENT:.1}% gate; \
                 exiting with status {EXIT_WALL_DRIFT}"
            );
            std::process::exit(EXIT_WALL_DRIFT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_boot_mode_defaults_to_firmware_set() {
        let args = vec!["cli".into(), "run-game".into()];
        assert_eq!(parse_boot_mode_inner(&args).unwrap(), BootMode::FirmwareSet);
    }

    #[test]
    fn parse_boot_mode_reads_explicit_single_prx() {
        let args = vec![
            "cli".into(),
            "run-game".into(),
            "--boot-mode".into(),
            "single-prx".into(),
        ];
        assert_eq!(parse_boot_mode_inner(&args).unwrap(), BootMode::SinglePrx);
    }

    #[test]
    fn parse_boot_mode_reads_firmware_set() {
        let args = vec![
            "cli".into(),
            "run-game".into(),
            "--boot-mode".into(),
            "firmware-set".into(),
        ];
        assert_eq!(parse_boot_mode_inner(&args).unwrap(), BootMode::FirmwareSet);
    }

    #[test]
    fn parse_boot_mode_rejects_unknown() {
        let args = vec![
            "cli".into(),
            "run-game".into(),
            "--boot-mode".into(),
            "wat".into(),
        ];
        let err = parse_boot_mode_inner(&args).unwrap_err();
        assert!(err.contains("unknown --boot-mode value"), "got: {err}");
    }

    #[test]
    fn cross_check_rejects_firmware_set_without_dir() {
        let err = cross_check_boot_mode_inner(BootMode::FirmwareSet, None).unwrap_err();
        assert!(err.contains("firmware-set"), "got: {err}");
        assert!(err.contains("--firmware-dir"), "got: {err}");
    }

    #[test]
    fn cross_check_allows_single_prx_with_or_without_dir() {
        assert!(cross_check_boot_mode_inner(BootMode::SinglePrx, None).is_ok());
        assert!(cross_check_boot_mode_inner(BootMode::SinglePrx, Some("foo")).is_ok());
    }

    #[test]
    fn cross_check_allows_firmware_set_with_dir() {
        assert!(cross_check_boot_mode_inner(BootMode::FirmwareSet, Some("foo")).is_ok());
    }

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
}
