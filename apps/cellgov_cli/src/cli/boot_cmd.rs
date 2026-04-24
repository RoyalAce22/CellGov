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
    let firmware_dir = find_flag_value(args, "--firmware-dir");
    let dump_at_pc = parse_hex_flag(args, "--dump-at-pc");
    let dump_skip: u32 = parse_flag_value(args, "--dump-skip").unwrap_or(0);
    let dump_mem_addrs: Vec<u64> = find_flag_value(args, "--dump-mem")
        .map(|v| {
            v.split(',')
                .map(|s| parse_hex_u64(s, "--dump-mem"))
                .collect()
        })
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
        dump_mem_addrs: &dump_mem_addrs,
        save_observation: save_observation.as_deref(),
        observation_manifest: observation_manifest.as_deref(),
        strict_reserved,
        profile_pairs,
        budget_override,
    });
}

pub(crate) fn bench_boot_once(args: &[String]) {
    let inputs = resolve_boot_inputs(args, "bench-boot-once");
    let max_steps: usize = parse_flag_value(args, "--max-steps").unwrap_or(100_000_000);
    let firmware_dir = find_flag_value(args, "--firmware-dir");
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
    let firmware_dir = find_flag_value(args, "--firmware-dir");
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
