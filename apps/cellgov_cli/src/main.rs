//! cellgov_cli -- run scenarios, dump traces, compare observations, explore schedules.

mod cli;
mod dump_imports;
mod game;

use cli::exit::die;
use cli::scenarios::{report, run_scenario, SCENARIOS};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        print_usage();
        std::process::exit(0);
    }

    match args[1].as_str() {
        "compare" => cli::compare::run(&args, SCENARIOS),
        "compare-observations" => {
            let a_path = args.get(2).map(String::as_str).unwrap_or_else(|| {
                die("usage: cellgov_cli compare-observations <a.json> <b.json>")
            });
            let b_path = args.get(3).map(String::as_str).unwrap_or_else(|| {
                die("usage: cellgov_cli compare-observations <a.json> <b.json>")
            });
            cli::compare::run_compare_observations(a_path, b_path);
        }
        "diverge" => {
            let a_path = args
                .get(2)
                .map(String::as_str)
                .unwrap_or_else(|| die("usage: cellgov_cli diverge <a.state> <b.state>"));
            let b_path = args
                .get(3)
                .map(String::as_str)
                .unwrap_or_else(|| die("usage: cellgov_cli diverge <a.state> <b.state>"));
            cli::compare::run_diverge(a_path, b_path);
        }
        "zoom" => {
            let a_path = args.get(2).map(String::as_str).unwrap_or_else(|| {
                die("usage: cellgov_cli zoom <a.zoom.state> <b.zoom.state> <step>")
            });
            let b_path = args.get(3).map(String::as_str).unwrap_or_else(|| {
                die("usage: cellgov_cli zoom <a.zoom.state> <b.zoom.state> <step>")
            });
            let step_str = args.get(4).map(String::as_str).unwrap_or_else(|| {
                die("usage: cellgov_cli zoom <a.zoom.state> <b.zoom.state> <step>")
            });
            let step: u64 = step_str
                .parse()
                .unwrap_or_else(|e| die(&format!("invalid step '{step_str}': {e}")));
            cli::compare::run_zoom(a_path, b_path, step);
        }
        "explore" => cli::explore::run(&args, SCENARIOS),
        "run-game" => cli::boot_cmd::run_game(&args),
        "bench-boot-once" => cli::boot_cmd::bench_boot_once(&args),
        "bench-boot" => cli::boot_cmd::bench_boot(&args),
        "dump" => cli::dump::run(&args, SCENARIOS),
        "dump-imports" => dump_imports::run(&args),
        other => match run_scenario(other) {
            Some((label, result)) => println!("{}", report(label, &result)),
            None => die(&format!(
                "unknown scenario: {other}\navailable: {}",
                SCENARIOS.join(", ")
            )),
        },
    }
}

fn print_usage() {
    println!("usage: cellgov_cli <scenario>");
    println!("       cellgov_cli dump <scenario>");
    println!("       cellgov_cli compare <scenario|manifest.toml> [--mode strict|memory|events|prefix] [--format human|json]");
    println!("       cellgov_cli compare <scenario|manifest.toml> --save-baseline <path>");
    println!(
        "       cellgov_cli compare <scenario|manifest.toml> --against-baseline <path> [--mode ...] [--format ...]"
    );
    println!(
        "       cellgov_cli compare <manifest.toml> --baselines-dir <dir> [--mode ...] [--format ...]"
    );
    println!("       cellgov_cli explore <scenario> [--format human|json]");
    println!("       cellgov_cli explore micro <name> [--format human|json]");
    println!(
        "       cellgov_cli run-game <elf-path> [--max-steps N] [--budget N] [--trace] [--profile]"
    );
    println!(
        "       cellgov_cli bench-boot --title <name> [--max-steps N] [--budget N] [--firmware-dir DIR]\n\
         \t\t[--checkpoint process-exit|first-rsx-write|pc=0xADDR]"
    );
    println!();
    println!("available scenarios:");
    for name in SCENARIOS {
        println!("  {name}");
    }
}
