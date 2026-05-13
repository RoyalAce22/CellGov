//! Dispatch for the boot-family subcommands: `run-game`,
//! `bench-boot-once`, and `bench-boot`. The guest-side pipeline
//! lives in the sibling `game` module.

use crate::game;

use super::args::{
    find_flag_value, find_run_game_elf_path, parse_flag_value, parse_hex_flag, parse_hex_u64,
    parse_patch_byte_pair,
};
use super::exit::die;
use super::title::{resolve_checkpoint_override, resolve_ps3_vfs_root, resolve_title_manifest};

/// Maximum allowed wall-time disagreement between the two bench-boot
/// subprocess runs, as a percentage of the faster run.
const AGREEMENT_GATE_PERCENT: f64 = 5.0;

/// Where `cellgov_firmware install` lands foundation SPRXes by default.
const DEFAULT_FIRMWARE_DIR: &str = "firmware/sys/external";

/// Set by synthetic harnesses (e.g. ps3autotests) to suppress the
/// auto-default.
const DISABLE_DEFAULT_ENV: &str = "CELLGOV_NO_FIRMWARE_DIR";

/// Explicit `--firmware-dir` wins; otherwise auto-default to
/// [`DEFAULT_FIRMWARE_DIR`] when it exists; `None` falls back to pure
/// HLE.
fn resolve_firmware_dir(args: &[String]) -> Option<String> {
    if let Some(explicit) = find_flag_value(args, "--firmware-dir") {
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

fn resolve_boot_inputs(args: &[String], subcmd: &str) -> BootInputs {
    let title = resolve_title_manifest(args, subcmd);
    let vfs_root = resolve_ps3_vfs_root(args);
    let elf_path = match find_run_game_elf_path(args) {
        Some(p) => p,
        None => match title.resolve_eboot(&vfs_root) {
            // Normalize path separators so logs read the same whether
            // components came from a string literal or a Windows PathBuf.
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(e) => die(&format!("{subcmd}: {e}")),
        },
    };
    BootInputs { title, elf_path }
}

pub(crate) fn run_game(args: &[String]) {
    let inputs = resolve_boot_inputs(args, "run-game");
    let max_steps: usize = parse_flag_value(args, "--max-steps").unwrap_or(100_000);
    let trace = args.iter().any(|a| a == "--trace");
    let profile = args.iter().any(|a| a == "--profile");
    let firmware_dir = resolve_firmware_dir(args);
    let dump_at_pc = parse_hex_flag(args, "--dump-at-pc");
    let dump_skip: u32 = parse_flag_value(args, "--dump-skip").unwrap_or(0);
    let dump_mem_boot_addrs: Vec<u64> = find_flag_value(args, "--dump-mem-boot")
        .map(|v| {
            v.split(',')
                .map(|s| parse_hex_u64(s, "--dump-mem-boot"))
                .collect()
        })
        .unwrap_or_default();
    let dump_mem_fault_ranges: Vec<(u64, u64)> = find_flag_value(args, "--dump-mem-fault")
        .map(|v| v.split(',').map(parse_dump_mem_fault_range).collect())
        .unwrap_or_default();
    let patch_bytes: Vec<(u64, u8)> = find_flag_value(args, "--patch-byte")
        .map(|v| v.split(',').map(parse_patch_byte_pair).collect())
        .unwrap_or_default();
    let save_observation = find_flag_value(args, "--save-observation");
    let observation_manifest = find_flag_value(args, "--observation-manifest");
    let strict_reserved = args.iter().any(|a| a == "--strict-reserved");
    let profile_pairs = args.iter().any(|a| a == "--profile-pairs");
    let budget_override: Option<u64> = parse_flag_value(args, "--budget");
    game::run_game(game::RunGameOptions {
        title: &inputs.title,
        elf_path: &inputs.elf_path,
        max_steps,
        trace,
        profile,
        firmware_dir: firmware_dir.as_deref(),
        dump_at_pc,
        dump_skip,
        patch_bytes: &patch_bytes,
        dump_mem_boot_addrs: &dump_mem_boot_addrs,
        dump_mem_fault_ranges: &dump_mem_fault_ranges,
        save_observation: save_observation.as_deref(),
        observation_manifest: observation_manifest.as_deref(),
        strict_reserved,
        profile_pairs,
        budget_override,
    });
}

/// Form: `0xADDR` (default 64 bytes) or `0xADDR:LEN`. LEN is clamped
/// to 64 KiB.
fn parse_dump_mem_fault_range(spec: &str) -> (u64, u64) {
    const DEFAULT_LEN: u64 = 64;
    const MAX_LEN: u64 = 64 * 1024;
    let (addr_str, len) = match spec.split_once(':') {
        Some((a, l)) => (a, parse_hex_u64(l, "--dump-mem-fault length")),
        None => (spec, DEFAULT_LEN),
    };
    let addr = parse_hex_u64(addr_str, "--dump-mem-fault address");
    let len = len.min(MAX_LEN);
    (addr, len)
}

pub(crate) fn bench_boot_once(args: &[String]) {
    let inputs = resolve_boot_inputs(args, "bench-boot-once");
    let max_steps: usize = parse_flag_value(args, "--max-steps").unwrap_or(100_000_000);
    let firmware_dir = resolve_firmware_dir(args);
    let strict_reserved = args.iter().any(|a| a == "--strict-reserved");
    let checkpoint_override = resolve_checkpoint_override(args, "bench-boot-once");
    let budget_override: Option<u64> = parse_flag_value(args, "--budget");
    game::bench_boot_one_run(
        &inputs.title,
        &inputs.elf_path,
        max_steps,
        firmware_dir.as_deref(),
        strict_reserved,
        checkpoint_override,
        budget_override,
    );
}

pub(crate) fn bench_boot(args: &[String]) {
    let inputs = resolve_boot_inputs(args, "bench-boot");
    let max_steps: usize = parse_flag_value(args, "--max-steps").unwrap_or(100_000_000);
    let firmware_dir = resolve_firmware_dir(args);
    let strict_reserved = args.iter().any(|a| a == "--strict-reserved");
    let checkpoint_override = resolve_checkpoint_override(args, "bench-boot");
    let budget_override: Option<u64> = parse_flag_value(args, "--budget");
    let (r1, r2) = game::bench_boot_pair(
        &inputs.title,
        &inputs.elf_path,
        max_steps,
        firmware_dir.as_deref(),
        strict_reserved,
        checkpoint_override,
        budget_override,
    );
    let agreement = game::agreement_percent(r1.wall, r2.wall);
    if agreement > AGREEMENT_GATE_PERCENT {
        eprintln!(
            "bench-boot: agreement {agreement:.2}% exceeds {AGREEMENT_GATE_PERCENT:.1}% gate; exiting with status 2"
        );
        std::process::exit(2);
    }
}
