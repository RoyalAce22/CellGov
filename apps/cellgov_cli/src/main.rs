//! cellgov_cli -- run scenarios, dump traces, compare observations, explore schedules.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "CLI binary: stdout/stderr are the user-facing output channel"
)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

mod cli;
mod disasm;
mod dump_imports;
mod dump_prx_imports;
mod game;

use cli::exit::die;
use cli::scenarios::{report, run_scenario, SCENARIOS};

/// Usage lines for the fixed-arity subcommands. Both the per-arm
/// `die` on wrong arity and the [`Subcommand::usage`] table reference
/// these consts so the two cannot drift.
const USAGE_COMPARE_OBSERVATIONS: &str =
    "cellgov_cli compare-observations <a.json> <b.json> [--format human|json]";
const USAGE_DIVERGE: &str = "cellgov_cli diverge <a.state> <b.state>";
const USAGE_ZOOM: &str = "cellgov_cli zoom <a.zoom.state> <b.zoom.state> <step>";

const USAGE_COMPARE: &str = "\
cellgov_cli compare <scenario|manifest.toml> [--mode strict|memory|events|prefix] [--format human|json]
cellgov_cli compare <scenario|manifest.toml> --save-baseline <path>
cellgov_cli compare <scenario|manifest.toml> --against-baseline <path> [--mode ...] [--format ...]
cellgov_cli compare <manifest.toml> --baselines-dir <dir> [--mode ...] [--format ...]";
const USAGE_EXPLORE: &str = "\
cellgov_cli explore <scenario> [--format human|json]
cellgov_cli explore micro <name> [--format human|json]";
const USAGE_RUN_GAME: &str = "\
cellgov_cli run-game <elf-path|--title NAME> [--max-steps N] [--budget N] [--trace] [--profile]
\t\t[--firmware-dir DIR] [--dump-mem-boot 0xADDR[,...]] [--dump-mem-fault 0xADDR[:LEN][,...]]
\t\t(default --firmware-dir: firmware/sys/external/ when present at the current working directory)";
const USAGE_BENCH_BOOT: &str = "\
cellgov_cli bench-boot --title <name> [--max-steps N] [--budget N] [--firmware-dir DIR]
\t\t[--checkpoint process-exit|first-rsx-write|pc=0xADDR]";
const USAGE_BENCH_BOOT_ONCE: &str = "\
cellgov_cli bench-boot-once <--title NAME|--content-id ID|--title-manifest PATH>
\t\t[--max-steps N] [--budget N] [--firmware-dir DIR]
\t\t[--checkpoint process-exit|first-rsx-write|pc=0xADDR]";
const USAGE_DUMP: &str = "cellgov_cli dump <scenario>";
const USAGE_DUMP_IMPORTS: &str =
    "cellgov_cli dump-imports <--title NAME|--content-id ID|--title-manifest PATH>";
const USAGE_DUMP_PRX_IMPORTS: &str =
    "cellgov_cli dump-prx-imports <path-to-prx-or-sprx> [--at 0xADDR] [--module NAME]";
const USAGE_DISASM: &str = "cellgov_cli disasm <elf-path> --vaddr <hex> [--count N]";
const USAGE_RPCS3_ATTRIBUTE: &str =
    "cellgov_cli rpcs3-attribute --trace <path> [--addr 0xADDR [--len N]] [--list] [--ranked]";
const USAGE_FIXTURE_GEN: &str = "\
cellgov_cli fixture-gen --manifest <path> --cellgov <path> --rpcs3 <path> --output-dir <path>
\t\t[--vfs-root PATH] (defaults: CELLGOV_PS3_VFS_ROOT env, then tools/rpcs3/dev_hdd0)";
const USAGE_TITLES_GEN: &str = "\
cellgov_cli titles-gen [--registry DIR] [--fixtures-dir DIR] [--output PATH]
\t\t(defaults: docs/titles, tests/fixtures, docs/titles.md)";

/// Top-level dispatcher routes. Adding a variant produces an
/// exhaustiveness error in [`Subcommand::tokens`], [`Subcommand::usage`],
/// and the `main` dispatch match, so the wiring stays in sync.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Subcommand {
    Help,
    Version,
    Compare,
    CompareObservations,
    Diverge,
    Zoom,
    Explore,
    RunGame,
    BenchBoot,
    BenchBootOnce,
    Dump,
    DumpImports,
    DumpPrxImports,
    Disasm,
    Rpcs3Attribute,
    FixtureGen,
    TitlesGen,
}

impl Subcommand {
    fn tokens(self) -> &'static [&'static str] {
        match self {
            Self::Help => &["--help", "-h", "help"],
            Self::Version => &["--version"],
            Self::Compare => &["compare"],
            Self::CompareObservations => &["compare-observations"],
            Self::Diverge => &["diverge"],
            Self::Zoom => &["zoom"],
            Self::Explore => &["explore"],
            Self::RunGame => &["run-game"],
            Self::BenchBoot => &["bench-boot"],
            Self::BenchBootOnce => &["bench-boot-once"],
            Self::Dump => &["dump"],
            Self::DumpImports => &["dump-imports"],
            Self::DumpPrxImports => &["dump-prx-imports"],
            Self::Disasm => &["disasm"],
            Self::Rpcs3Attribute => &["rpcs3-attribute"],
            Self::FixtureGen => &["fixture-gen"],
            Self::TitlesGen => &["titles-gen"],
        }
    }

    /// Usage block printed by [`print_usage`]. May contain embedded
    /// `\n` for subcommands that document several invocation shapes.
    /// `None` for variants rolled into the trailing summary line.
    fn usage(self) -> Option<&'static str> {
        match self {
            Self::Help | Self::Version => None,
            Self::Compare => Some(USAGE_COMPARE),
            Self::CompareObservations => Some(USAGE_COMPARE_OBSERVATIONS),
            Self::Diverge => Some(USAGE_DIVERGE),
            Self::Zoom => Some(USAGE_ZOOM),
            Self::Explore => Some(USAGE_EXPLORE),
            Self::RunGame => Some(USAGE_RUN_GAME),
            Self::BenchBoot => Some(USAGE_BENCH_BOOT),
            Self::BenchBootOnce => Some(USAGE_BENCH_BOOT_ONCE),
            Self::Dump => Some(USAGE_DUMP),
            Self::DumpImports => Some(USAGE_DUMP_IMPORTS),
            Self::DumpPrxImports => Some(USAGE_DUMP_PRX_IMPORTS),
            Self::Disasm => Some(USAGE_DISASM),
            Self::Rpcs3Attribute => Some(USAGE_RPCS3_ATTRIBUTE),
            Self::FixtureGen => Some(USAGE_FIXTURE_GEN),
            Self::TitlesGen => Some(USAGE_TITLES_GEN),
        }
    }

    fn from_token(t: &str) -> Option<Self> {
        SUBCOMMANDS
            .iter()
            .copied()
            .find(|s| s.tokens().contains(&t))
    }
}

/// Canonical iteration order. Drives [`print_usage`] layout and the
/// unknown-token diagnostic. The `subcommands_const_is_exhaustive`
/// test pins this list against the [`Subcommand`] variants so a
/// newly-added variant cannot be silently absent.
const SUBCOMMANDS: &[Subcommand] = &[
    Subcommand::Help,
    Subcommand::Version,
    Subcommand::Compare,
    Subcommand::CompareObservations,
    Subcommand::Diverge,
    Subcommand::Zoom,
    Subcommand::Explore,
    Subcommand::RunGame,
    Subcommand::BenchBoot,
    Subcommand::BenchBootOnce,
    Subcommand::Dump,
    Subcommand::DumpImports,
    Subcommand::DumpPrxImports,
    Subcommand::Disasm,
    Subcommand::Rpcs3Attribute,
    Subcommand::FixtureGen,
    Subcommand::TitlesGen,
];

fn main() {
    debug_assert!(
        SCENARIOS
            .iter()
            .all(|s| Subcommand::from_token(s).is_none()),
        "scenario name collides with a dispatcher token"
    );

    let args = collect_args_or_die();
    if args.len() < 2 {
        print_usage();
        die("missing subcommand or scenario");
    }

    let token = args[1].as_str();
    match Subcommand::from_token(token) {
        Some(Subcommand::Help) => print_usage(),
        Some(Subcommand::Version) => println!("cellgov_cli {}", env!("CARGO_PKG_VERSION")),
        Some(Subcommand::Compare) => cli::compare::run(&args, SCENARIOS),
        Some(Subcommand::CompareObservations) => {
            if args.len() < 4 {
                die(USAGE_COMPARE_OBSERVATIONS);
            }
            cli::compare::run_compare_observations(&args);
        }
        Some(Subcommand::Diverge) => {
            if args.len() != 4 {
                die(USAGE_DIVERGE);
            }
            cli::compare::run_diverge(&args[2], &args[3]);
        }
        Some(Subcommand::Zoom) => {
            if args.len() != 5 {
                die(USAGE_ZOOM);
            }
            let step_str = &args[4];
            let step = parse_step_count(step_str)
                .unwrap_or_else(|e| die(&format!("invalid step '{step_str}': {e}")));
            cli::compare::run_zoom(&args[2], &args[3], step);
        }
        Some(Subcommand::Explore) => cli::explore::run(&args, SCENARIOS),
        Some(Subcommand::RunGame) => cli::boot_cmd::run_game(&args),
        Some(Subcommand::BenchBoot) => cli::boot_cmd::bench_boot(&args),
        Some(Subcommand::BenchBootOnce) => cli::boot_cmd::bench_boot_once(&args),
        Some(Subcommand::Dump) => cli::dump::run(&args, SCENARIOS),
        Some(Subcommand::DumpImports) => dump_imports::run(&args),
        Some(Subcommand::DumpPrxImports) => dump_prx_imports::run(&args),
        Some(Subcommand::Disasm) => disasm::run(&args),
        Some(Subcommand::Rpcs3Attribute) => cli::rpcs3_attribute::run(&args),
        Some(Subcommand::FixtureGen) => cli::fixture_gen::run(&args),
        Some(Subcommand::TitlesGen) => cli::titles_gen::run(&args),
        None => match run_scenario(token) {
            Some((label, result)) => println!("{}", report(label, &result)),
            None => die(&format!(
                "unknown subcommand or scenario: {token}\n\
                 available subcommands: {}\n\
                 available scenarios: {}",
                all_subcommand_tokens().join(", "),
                SCENARIOS.join(", "),
            )),
        },
    }
}

/// Materialize argv as `Vec<String>`, dying with a structured error on
/// non-UTF-8 arguments instead of letting `std::env::args` panic.
fn collect_args_or_die() -> Vec<String> {
    let mut out = Vec::new();
    for (i, raw) in std::env::args_os().enumerate() {
        match raw.into_string() {
            Ok(s) => out.push(s),
            Err(os) => die(&format!(
                "argv[{i}]: not valid UTF-8 ({os:?}); cellgov_cli accepts only UTF-8 arguments"
            )),
        }
    }
    out
}

/// Parse a step count for `zoom`. Accepts `0x`/`0X` hex per the rest
/// of the CLI's address-shaped flags; otherwise decimal.
fn parse_step_count(s: &str) -> Result<u64, std::num::ParseIntError> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16)
    } else {
        s.parse()
    }
}

/// Every dispatcher-recognized token across all subcommands, in
/// `SUBCOMMANDS` order. Backs the unknown-token diagnostic so the
/// list mirrors whatever the dispatcher will actually accept.
fn all_subcommand_tokens() -> Vec<&'static str> {
    SUBCOMMANDS
        .iter()
        .flat_map(|s| s.tokens().iter().copied())
        .collect()
}

fn print_usage() {
    let mut lines: Vec<&str> = Vec::new();
    lines.push("cellgov_cli <scenario>");
    for sub in SUBCOMMANDS {
        if let Some(text) = sub.usage() {
            lines.extend(text.lines());
        }
    }
    lines.push("cellgov_cli --help | --version");
    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            println!("usage: {line}");
        } else {
            println!("       {line}");
        }
    }
    println!();
    println!("available scenarios:");
    for name in SCENARIOS {
        println!("  {name}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_step_count_accepts_decimal() {
        assert_eq!(parse_step_count("123").unwrap(), 123);
    }

    #[test]
    fn parse_step_count_accepts_lower_hex_prefix() {
        assert_eq!(parse_step_count("0xff").unwrap(), 0xff);
    }

    #[test]
    fn parse_step_count_accepts_upper_hex_prefix() {
        assert_eq!(parse_step_count("0XFF").unwrap(), 0xff);
    }

    #[test]
    fn parse_step_count_rejects_garbage() {
        assert!(parse_step_count("nope").is_err());
        assert!(parse_step_count("0xnope").is_err());
    }

    #[test]
    fn subcommands_const_is_exhaustive() {
        for sub in SUBCOMMANDS {
            for tok in sub.tokens() {
                assert_eq!(
                    Subcommand::from_token(tok),
                    Some(*sub),
                    "token {tok:?} did not round-trip to {sub:?}"
                );
            }
        }
        let expected: &[Subcommand] = &[
            Subcommand::Help,
            Subcommand::Version,
            Subcommand::Compare,
            Subcommand::CompareObservations,
            Subcommand::Diverge,
            Subcommand::Zoom,
            Subcommand::Explore,
            Subcommand::RunGame,
            Subcommand::BenchBoot,
            Subcommand::BenchBootOnce,
            Subcommand::Dump,
            Subcommand::DumpImports,
            Subcommand::DumpPrxImports,
            Subcommand::Disasm,
            Subcommand::Rpcs3Attribute,
            Subcommand::FixtureGen,
            Subcommand::TitlesGen,
        ];
        assert_eq!(
            SUBCOMMANDS.len(),
            expected.len(),
            "SUBCOMMANDS missing a variant present in `expected`"
        );
        for (a, b) in SUBCOMMANDS.iter().zip(expected.iter()) {
            assert_eq!(a, b, "SUBCOMMANDS ordering drifted from `expected`");
        }
    }

    #[test]
    fn tokens_have_no_duplicates_across_variants() {
        let mut seen: std::collections::BTreeMap<&str, Subcommand> =
            std::collections::BTreeMap::new();
        for sub in SUBCOMMANDS {
            for tok in sub.tokens() {
                if let Some(prev) = seen.insert(*tok, *sub) {
                    panic!(
                        "token {tok:?} claimed by both {prev:?} and {sub:?}",
                        prev = prev,
                        sub = sub
                    );
                }
            }
        }
    }

    #[test]
    fn scenarios_disjoint_from_subcommand_tokens() {
        for s in SCENARIOS {
            assert!(
                Subcommand::from_token(s).is_none(),
                "scenario {s:?} shadowed by dispatcher token"
            );
        }
    }

    #[test]
    fn from_token_returns_none_for_unknown() {
        assert_eq!(Subcommand::from_token(""), None);
        assert_eq!(Subcommand::from_token("nope"), None);
        assert_eq!(Subcommand::from_token("compaer"), None);
    }
}
