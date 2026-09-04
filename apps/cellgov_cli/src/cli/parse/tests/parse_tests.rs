//! The command tree, driven through `try_parse_from`.

use clap::{CommandFactory, Parser};

use super::globals::{reads_format, reads_vfs_root, renders_progress};
use super::*;

/// Parse one invocation, with the program name prepended.
fn parse(argv: &[&str]) -> Result<Cli, clap::Error> {
    let mut full = vec!["cellgov"];
    full.extend_from_slice(argv);
    Cli::try_parse_from(full)
}

fn err_kind(argv: &[&str]) -> clap::error::ErrorKind {
    parse(argv).expect_err("expected a usage error").kind()
}

#[test]
fn the_tree_is_internally_consistent() {
    Cli::command().debug_assert();
}

#[test]
fn the_binary_is_named_after_the_command_operators_type() {
    assert_eq!(Cli::command().get_name(), "cellgov");
}

/// One invocation of every leaf in the command tree.
const EVERY_LEAF: &[&[&str]] = &[
    &["status"],
    &["firmware", "install", "fw.pup"],
    &["firmware", "list"],
    &["firmware", "show", "4.91"],
    &["firmware", "verify", "4.91"],
    &["firmware", "uninstall", "4.91"],
    &["title", "list"],
    &["title", "show", "NPAA00001"],
    &["title", "verify", "NPAA00001"],
    &["title", "install", "game.pkg"],
    &["title", "install-update", "patch.pkg"],
    &["title", "uninstall", "NPAA00001"],
    &["keys", "show"],
    &["keys", "import", "keys.txt"],
    &["keys", "remove"],
    &["self", "decrypt", "EBOOT.BIN"],
    &["boot", "run", "--title", "synthetic"],
    &["boot", "bench", "--title", "synthetic"],
    &["boot", "bench-once", "--title", "synthetic"],
    &["diff", "compare", "fairness"],
    &["diff", "observations", "a.json", "b.json"],
    &["diff", "diverge", "a.state", "b.state"],
    &["diff", "zoom", "a.state", "b.state", "12"],
    &["explore", "fairness"],
    &["explore", "micro", "ppu_hello"],
    &["scenario", "list"],
    &["scenario", "run", "fairness"],
    &["scenario", "dump", "fairness"],
    &["dev", "disasm", "e.elf", "--vaddr", "0x10000"],
    &["dev", "prx-imports", "m.sprx"],
    &["dev", "funcs", "e.elf"],
    &["dev", "rpcs3-attribute", "--trace", "t.htrc", "--list"],
    &[
        "dev",
        "fixture-gen",
        "--manifest",
        "m.toml",
        "--cellgov",
        "a.json",
        "--rpcs3",
        "b.json",
        "--output-dir",
        "out",
    ],
    &["dev", "titles-gen"],
    &["dev", "gen-manifest", "--title-id", "NPAA00001"],
    &["dev", "record-anchors", "--all"],
];

#[test]
fn every_leaf_parses() {
    for argv in EVERY_LEAF {
        assert!(parse(argv).is_ok(), "{argv:?}");
    }
}

#[test]
fn every_leaf_answers_help() {
    for argv in EVERY_LEAF {
        // `--help` short-circuits before the tree reaches the operands.
        let path: Vec<&str> = argv
            .iter()
            .copied()
            .take_while(|t| !t.starts_with('-'))
            .collect();
        let mut with_help = path.clone();
        with_help.push("--help");
        assert_eq!(
            err_kind(&with_help),
            clap::error::ErrorKind::DisplayHelp,
            "{path:?}"
        );
    }
}

// -- required arguments and conflicts --

#[test]
fn a_boot_needs_exactly_one_title_selector() {
    assert_eq!(
        err_kind(&["boot", "run"]),
        clap::error::ErrorKind::MissingRequiredArgument
    );
    assert_eq!(
        err_kind(&[
            "boot",
            "run",
            "--title",
            "synthetic",
            "--content-id",
            "NPAA00001"
        ]),
        clap::error::ErrorKind::ArgumentConflict
    );
}

#[test]
fn a_firmware_selection_names_the_store_or_a_tree_but_not_both() {
    assert_eq!(
        err_kind(&[
            "boot",
            "bench",
            "--title",
            "synthetic",
            "--fw",
            "4.91",
            "--firmware-dir",
            "d"
        ]),
        clap::error::ErrorKind::ArgumentConflict
    );
}

#[test]
fn recording_a_baseline_runs_no_comparison_so_it_refuses_the_report_flags() {
    for conflicting in [
        vec!["--mode", "strict"],
        vec!["--format", "json"],
        vec!["--against-baseline", "b.json"],
        vec!["--observations-dir", "d"],
    ] {
        let mut argv = vec!["diff", "compare", "fairness", "--save-baseline", "b.json"];
        argv.extend(conflicting.iter().copied());
        assert_eq!(
            err_kind(&argv),
            clap::error::ErrorKind::ArgumentConflict,
            "{conflicting:?}"
        );
    }
}

#[test]
fn an_attribute_query_names_a_mode() {
    assert_eq!(
        err_kind(&["dev", "rpcs3-attribute", "--trace", "t.htrc"]),
        clap::error::ErrorKind::MissingRequiredArgument
    );
    assert_eq!(
        err_kind(&["dev", "rpcs3-attribute", "--list"]),
        clap::error::ErrorKind::MissingRequiredArgument
    );
    assert_eq!(
        err_kind(&["dev", "rpcs3-attribute", "--trace", "t.htrc", "--len", "8"]),
        clap::error::ErrorKind::MissingRequiredArgument,
        "--len describes --addr and says nothing on its own",
    );
}

#[test]
fn record_anchors_takes_all_or_one_title_and_not_both() {
    assert_eq!(
        err_kind(&["dev", "record-anchors"]),
        clap::error::ErrorKind::MissingRequiredArgument
    );
    assert_eq!(
        err_kind(&["dev", "record-anchors", "--all", "--title", "synthetic"]),
        clap::error::ErrorKind::ArgumentConflict
    );
}

#[test]
fn gen_manifest_takes_a_record_or_a_title_id_and_not_both() {
    assert_eq!(
        err_kind(&["dev", "gen-manifest"]),
        clap::error::ErrorKind::MissingRequiredArgument
    );
    assert_eq!(
        err_kind(&[
            "dev",
            "gen-manifest",
            "--record",
            "r.toml",
            "--title-id",
            "X"
        ]),
        clap::error::ErrorKind::ArgumentConflict
    );
    assert_eq!(
        err_kind(&[
            "dev",
            "gen-manifest",
            "--record",
            "r.toml",
            "--installs",
            "d"
        ]),
        clap::error::ErrorKind::ArgumentConflict,
        "--installs only resolves a --title-id",
    );
}

#[test]
fn a_dump_skip_without_a_pc_to_skip_is_refused() {
    for skip in ["0", "3"] {
        assert_eq!(
            err_kind(&["boot", "run", "--title", "synthetic", "--dump-skip", skip]),
            clap::error::ErrorKind::MissingRequiredArgument,
            "--dump-skip {skip}"
        );
    }
}

#[test]
fn a_comma_list_flag_reaches_the_command_as_parsed_entries() {
    let cli = parse(&[
        "boot",
        "run",
        "--title",
        "synthetic",
        "--dump-mem-boot",
        "0x1000,0x2000",
        "--dump-mem-fault",
        "0x3000:8,0x4000",
        "--patch-byte",
        "0x5000=ff,0x5001=0",
    ])
    .unwrap();
    let Command::Boot(BootCommand::Run(args)) = cli.command else {
        panic!("expected boot run");
    };
    assert_eq!(
        args.dump_mem_boot.as_deref(),
        Some(&[0x1000u64, 0x2000][..])
    );
    assert_eq!(
        args.dump_mem_fault.as_deref(),
        Some(&[(0x3000u64, 8u64), (0x4000, 0x40)][..])
    );
    assert_eq!(
        args.patch_byte.as_deref(),
        Some(&[(0x5000u64, 0xffu8), (0x5001, 0)][..])
    );
}

#[test]
fn a_comma_list_flag_refuses_an_empty_entry() {
    for argv in [
        vec!["--dump-mem-boot", "0x1000,"],
        vec!["--dump-mem-boot", ",0x1000"],
        vec!["--dump-mem-fault", "0x1000,,0x2000"],
        vec!["--patch-byte", "0x1000=ff,"],
    ] {
        let mut full = vec!["boot", "run", "--title", "synthetic"];
        full.extend(argv.iter().copied());
        assert_eq!(
            err_kind(&full),
            clap::error::ErrorKind::ValueValidation,
            "{argv:?}"
        );
    }
}

#[test]
fn an_observation_manifest_without_an_observation_is_refused() {
    assert_eq!(
        err_kind(&[
            "boot",
            "run",
            "--title",
            "synthetic",
            "--observation-manifest",
            "m.toml"
        ]),
        clap::error::ErrorKind::MissingRequiredArgument
    );
}

#[test]
fn the_anchor_gate_is_dropped_only_where_there_is_one() {
    assert!(parse(&["boot", "bench", "--title", "synthetic", "--no-anchor-check"]).is_ok());
    assert_eq!(
        err_kind(&[
            "boot",
            "bench-once",
            "--title",
            "synthetic",
            "--no-anchor-check"
        ]),
        clap::error::ErrorKind::UnknownArgument
    );
}

#[test]
fn the_run_set_flags_reach_only_the_command_that_has_a_set() {
    assert!(parse(&[
        "boot",
        "bench",
        "--title",
        "t",
        "--runs",
        "5",
        "--strict-perf"
    ])
    .is_ok());
    for flag in [vec!["--runs", "5"], vec!["--strict-perf"]] {
        let mut argv = vec!["boot", "bench-once", "--title", "t"];
        argv.extend_from_slice(&flag);
        assert_eq!(
            err_kind(&argv),
            clap::error::ErrorKind::UnknownArgument,
            "{flag:?}"
        );
    }
}

#[test]
fn a_bench_takes_three_runs_unless_it_is_told_otherwise() {
    let cli = parse(&["boot", "bench", "--title", "t"]).unwrap();
    let Command::Boot(BootCommand::Bench(args)) = cli.command else {
        panic!("expected boot bench");
    };
    assert_eq!(args.runs, crate::game::BENCH_DEFAULT_RUNS);
    assert!(!args.strict_perf);
}

#[test]
fn a_run_count_outside_the_window_is_refused() {
    for count in ["0", "26"] {
        assert_eq!(
            err_kind(&["boot", "bench", "--title", "t", "--runs", count]),
            clap::error::ErrorKind::ValueValidation,
            "--runs {count}"
        );
    }
    assert!(parse(&["boot", "bench", "--title", "t", "--runs", "25"]).is_ok());
}

/// The parser admits `1`; the set reports that it gated nothing.
#[test]
fn a_set_of_one_run_is_inside_the_window() {
    let cli = parse(&["boot", "bench", "--title", "t", "--runs", "1"]).unwrap();
    let Command::Boot(BootCommand::Bench(args)) = cli.command else {
        panic!("expected boot bench");
    };
    assert_eq!(args.runs, 1);
}

#[test]
fn a_run_count_that_is_not_a_decimal_is_refused() {
    for raw in ["", "three", "3.5", "1e2", "-1", "0x3", " 3"] {
        // The attached form, so a leading '-' reaches the parser as a
        // value.
        let flag = format!("--runs={raw}");
        assert_eq!(
            err_kind(&["boot", "bench", "--title", "t", flag.as_str()]),
            clap::error::ErrorKind::ValueValidation,
            "--runs {raw:?}"
        );
    }
}

/// Both bench commands share `BenchArgs`, so clap parses these on the
/// set. `boot_cmd::bench_boot` refuses them, and
/// `tests/exit_code_contract.rs` covers that refusal.
#[test]
fn the_child_only_flags_still_parse_on_the_set() {
    assert!(parse(&[
        "boot",
        "bench",
        "--title",
        "t",
        "--save-state-trace",
        "t.state",
        "--run-index",
        "2",
    ])
    .is_ok());
}

#[test]
fn a_store_root_is_named_once() {
    assert_eq!(
        err_kind(&[
            "title",
            "install",
            "g.pkg",
            "--output",
            "store",
            "--vfs-root",
            "store/dev_hdd0"
        ]),
        clap::error::ErrorKind::ArgumentConflict
    );
}

/// The refusal `global_refusal` makes for `argv`, or `None`.
fn refusal(argv: &[&str]) -> Option<String> {
    global_refusal(&parse(argv).expect("clap accepts this invocation"))
}

#[test]
fn a_store_root_named_twice_is_refused_from_either_side_of_the_command() {
    let said = refusal(&[
        "--vfs-root",
        "store/dev_hdd0",
        "title",
        "install",
        "g.pkg",
        "--output",
        "store",
    ])
    .expect("clap alone does not catch the leading spelling");
    assert!(
        said.contains("--output") && said.contains("--vfs-root"),
        "{said}"
    );
}

#[test]
fn a_global_the_command_never_reads_is_refused_naming_it() {
    for (argv, flag) in [
        (
            vec!["--vfs-root", "v", "diff", "diverge", "a", "b"],
            "--vfs-root",
        ),
        (
            vec!["--format", "json", "boot", "bench", "--title", "synthetic"],
            "--format",
        ),
        (vec!["--quiet", "diff", "diverge", "a", "b"], "--quiet"),
        (
            vec!["--verbose", "boot", "bench", "--title", "synthetic"],
            "--verbose",
        ),
    ] {
        let said = refusal(&argv).unwrap_or_else(|| panic!("{flag} was accepted on {argv:?}"));
        assert!(said.starts_with(flag), "{said}");
    }
}

#[test]
fn a_global_the_command_reads_passes_the_check() {
    for argv in [
        vec!["--vfs-root", "v", "boot", "bench", "--title", "synthetic"],
        vec!["--format", "json", "diff", "compare", "fairness"],
        vec!["--quiet", "firmware", "install", "fw.pup"],
        vec!["--verbose", "firmware", "install", "fw.pup"],
        vec!["--quiet", "boot", "bench", "--title", "synthetic"],
        vec!["--quiet", "boot", "run", "--title", "synthetic"],
        vec!["--quiet", "boot", "bench-once", "--title", "synthetic"],
        vec!["--quiet", "dev", "record-anchors", "--all"],
        vec![
            "--no-color",
            "--no-progress",
            "--no-input",
            "--yes",
            "scenario",
            "list",
        ],
    ] {
        assert_eq!(refusal(&argv), None, "{argv:?}");
    }
}

// -- flag conventions --

#[test]
fn a_repeated_scalar_flag_is_refused() {
    for argv in [
        vec![
            "boot",
            "bench",
            "--title",
            "synthetic",
            "--fw",
            "3.55",
            "--fw",
            "4.91",
        ],
        vec!["dev", "prx-imports", "m.sprx", "--at", "0x1", "--at", "0x2"],
        vec![
            "diff", "compare", "fairness", "--mode", "strict", "--mode", "events",
        ],
    ] {
        assert_eq!(
            err_kind(&argv),
            clap::error::ErrorKind::ArgumentConflict,
            "{argv:?}"
        );
    }
}

#[test]
fn the_equals_spelling_is_accepted() {
    let cli = parse(&["boot", "bench", "--title=synthetic", "--fw=4.91"]).unwrap();
    let Command::Boot(BootCommand::Bench(args)) = cli.command else {
        panic!("expected boot bench");
    };
    assert_eq!(args.bench.selector.title.as_deref(), Some("synthetic"));
    assert_eq!(args.bench.selection.fw.as_deref(), Some("4.91"));
}

#[test]
fn a_guest_argument_may_spell_a_host_flag() {
    let cli = parse(&[
        "boot",
        "run",
        "--title",
        "synthetic",
        "--guest-arg",
        "--trace",
        "--guest-arg",
        "--fw",
    ])
    .unwrap();
    let Command::Boot(BootCommand::Run(args)) = cli.command else {
        panic!("expected boot run");
    };
    assert_eq!(args.guest_arg, vec!["--trace", "--fw"]);
    assert!(
        !args.trace,
        "the guest value must not switch host tracing on"
    );
    assert_eq!(args.selection.fw, None);
}

#[test]
fn a_global_flag_is_accepted_before_and_after_the_command() {
    let root = std::path::Path::new("vfs/dev_hdd0");
    let before = parse(&[
        "--vfs-root",
        "vfs/dev_hdd0",
        "boot",
        "bench",
        "--title",
        "synthetic",
    ])
    .unwrap();
    let after = parse(&[
        "boot",
        "bench",
        "--title",
        "synthetic",
        "--vfs-root",
        "vfs/dev_hdd0",
    ])
    .unwrap();
    let middle = parse(&[
        "boot",
        "--vfs-root",
        "vfs/dev_hdd0",
        "bench",
        "--title",
        "synthetic",
    ])
    .unwrap();
    for cli in [&before, &after, &middle] {
        assert_eq!(cli.globals.vfs_root.as_deref(), Some(root));
    }
}

#[test]
fn every_global_flag_parses_on_a_leaf() {
    let cli = parse(&[
        "boot",
        "bench",
        "--title",
        "synthetic",
        "--vfs-root",
        "vfs/dev_hdd0",
        "--quiet",
        "--verbose",
        "--no-color",
        "--no-progress",
        "--no-input",
        "--yes",
    ])
    .unwrap();
    let g = &cli.globals;
    assert_eq!(
        g.vfs_root.as_deref(),
        Some(std::path::Path::new("vfs/dev_hdd0"))
    );
    assert!(g.quiet && g.verbose && g.no_color && g.no_progress && g.no_input && g.yes);
}

#[test]
fn the_render_decision_is_never_in_machine_mode() {
    let json = parse(&["diff", "compare", "fairness", "--format", "json"]).unwrap();
    assert!(!json.globals.render().json);
    assert!(!reads_format(
        &parse(&["firmware", "install", "fw.pup"]).unwrap().command
    ));
    assert!(renders_progress(
        &parse(&["firmware", "install", "fw.pup"]).unwrap().command
    ));
}

#[test]
fn a_global_that_forbids_an_absent_behaviour_is_accepted_anywhere() {
    for flag in ["--no-color", "--no-progress", "--no-input", "--yes"] {
        assert_eq!(
            refusal(&["diff", "diverge", "a", "b", flag]),
            None,
            "{flag}"
        );
    }
}

#[test]
fn a_global_that_promises_output_is_refused_where_there_is_none() {
    assert!(refusal(&["diff", "diverge", "a", "b", "--quiet"]).is_some());
    assert!(refusal(&["dev", "titles-gen", "--quiet"]).is_some());
    // The boot family answers `--no-progress`, never `--verbose`.
    assert!(refusal(&["boot", "bench", "--title", "synthetic", "--verbose"]).is_some());
    assert!(refusal(&["boot", "run", "--title", "synthetic", "--verbose"]).is_some());
    assert_eq!(
        refusal(&["firmware", "install", "fw.pup", "--verbose"]),
        None
    );
}

#[test]
fn a_bad_value_is_a_usage_error_not_a_run() {
    for argv in [
        vec!["dev", "disasm", "e.elf", "--vaddr", "zz"],
        vec![
            "dev", "disasm", "e.elf", "--vaddr", "0x1000", "--count", "0",
        ],
        vec![
            "dev", "disasm", "e.elf", "--vaddr", "0x1000", "--count", "70000",
        ],
        vec![
            "boot",
            "bench",
            "--title",
            "synthetic",
            "--checkpoint",
            "halt",
        ],
        vec!["diff", "zoom", "a", "b", "later"],
    ] {
        assert_eq!(
            err_kind(&argv),
            clap::error::ErrorKind::ValueValidation,
            "{argv:?}"
        );
    }
    // An enum-valued flag names the values it does take instead.
    assert_eq!(
        err_kind(&["diff", "compare", "fairness", "--mode", "loose"]),
        clap::error::ErrorKind::InvalidValue
    );
}

#[test]
fn a_disasm_count_takes_the_ceiling_and_refuses_the_step_past_it() {
    let at_ceiling = MAX_DISASM_COUNT.to_string();
    let past_ceiling = (MAX_DISASM_COUNT + 1).to_string();
    let cli = parse(&[
        "dev",
        "disasm",
        "e.elf",
        "--vaddr",
        "0x1000",
        "--count",
        at_ceiling.as_str(),
    ])
    .unwrap();
    let Command::Dev(DevCommand::Disasm(args)) = cli.command else {
        panic!("expected dev disasm");
    };
    assert_eq!(args.count, MAX_DISASM_COUNT);
    assert_eq!(
        err_kind(&[
            "dev",
            "disasm",
            "e.elf",
            "--vaddr",
            "0x1000",
            "--count",
            past_ceiling.as_str(),
        ]),
        clap::error::ErrorKind::ValueValidation
    );
}

// -- the globals a command would parse and never read --

#[test]
fn a_vfs_root_is_refused_where_nothing_reads_one() {
    for command in [
        Command::Diff(DiffCommand::Diverge {
            a: "a".into(),
            b: "b".into(),
        }),
        Command::Scenario(ScenarioCommand::List),
    ] {
        assert!(!reads_vfs_root(&command));
    }
}

#[test]
fn a_vfs_root_reaches_every_command_that_opens_a_guest_image() {
    for argv in [
        vec!["boot", "run", "--title", "synthetic"],
        vec!["dev", "disasm", "e.elf", "--vaddr", "0x1000"],
        vec!["dev", "prx-imports", "m.sprx"],
        vec!["dev", "funcs", "e.elf"],
        vec!["self", "decrypt", "EBOOT.BIN"],
    ] {
        let cli = parse(&argv).unwrap();
        assert!(reads_vfs_root(&cli.command), "{argv:?}");
    }
}

#[test]
fn the_vfs_root_refusal_names_status_among_the_readers() {
    assert_eq!(refusal(&["--vfs-root", "vfs/dev_hdd0", "status"]), None);
    let said = refusal(&["--vfs-root", "vfs/dev_hdd0", "diff", "diverge", "a", "b"])
        .expect("diff diverge reads no vfs root");
    assert!(said.contains("status"), "{said}");
}

#[test]
fn a_format_is_read_only_where_a_report_is_rendered() {
    let rendered = [
        vec!["diff", "compare", "fairness"],
        vec!["diff", "observations", "a.json", "b.json"],
        vec!["explore", "fairness"],
    ];
    for argv in rendered {
        let cli = parse(&argv).unwrap();
        assert!(reads_format(&cli.command), "{argv:?}");
    }
    for argv in [
        vec!["boot", "bench", "--title", "synthetic"],
        vec!["diff", "diverge", "a", "b"],
        vec!["dev", "titles-gen"],
    ] {
        let cli = parse(&argv).unwrap();
        assert!(!reads_format(&cli.command), "{argv:?}");
    }
}

// -- help text --

/// The rendered long help of the command reached by `path`.
///
/// clap propagates a global argument into its subcommands during the
/// build, so this helper builds the tree before it walks `path`.
fn help_of(path: &[&str]) -> String {
    let mut command = Cli::command();
    command.build();
    for name in path {
        command = command
            .find_subcommand(name)
            .unwrap_or_else(|| panic!("no subcommand {name} under {path:?}"))
            .clone();
    }
    command.render_long_help().to_string()
}

#[test]
fn a_leaf_help_shows_the_globals_that_reach_it() {
    let help = help_of(&["dev", "disasm"]);
    assert!(
        help.contains("--vfs-root"),
        "a built leaf renders the globals propagated into it:
{help}"
    );
}

#[test]
fn the_top_level_help_states_the_exit_code_contract() {
    let help = Cli::command().render_long_help().to_string();
    for line in [
        "0    success",
        "2    usage error",
        "5    a boot moved off",
        ">=10",
    ] {
        assert!(help.contains(line), "top-level help is missing {line:?}");
    }
}

#[test]
fn every_command_that_reads_an_sce_input_carries_the_shared_note() {
    for path in [
        vec!["dev", "disasm"],
        vec!["dev", "prx-imports"],
        vec!["dev", "funcs"],
        vec!["self", "decrypt"],
    ] {
        let help = help_of(&path);
        assert!(
            help.contains(crate::cli::exit::SCE_INPUT_USAGE_NOTE),
            "{path:?} help is missing the SCE input note:\n{help}"
        );
    }
}

#[test]
fn the_sce_note_claims_a_decrypt_path_only_when_the_build_has_one() {
    let help = help_of(&["dev", "disasm"]);
    let has_decrypt = cfg!(feature = "decrypt");
    for claim in crate::cli::exit::DECRYPTION_CLAIMS {
        assert_eq!(
            help.contains(claim),
            has_decrypt,
            "disasm help and the build disagree over '{claim}':\n{help}"
        );
    }
}

mod uninstall_scope_tests {
    use super::*;

    /// Parse a `title uninstall` invocation carrying `argv`.
    fn uninstall(argv: &[&str]) -> Result<Cli, clap::Error> {
        let mut full = vec!["title", "uninstall", "NPAA00001"];
        full.extend_from_slice(argv);
        super::parse(&full)
    }

    #[test]
    fn two_scope_flags_are_refused_before_a_scope_is_chosen() {
        for argv in [
            vec!["--ver", "02.51", "--updates"],
            vec!["--ver", "02.51", "--all"],
            vec!["--updates", "--all"],
            vec!["--ver", "02.51", "--updates", "--all"],
        ] {
            let kind = uninstall(&argv).expect_err("expected a usage error").kind();
            assert_eq!(kind, clap::error::ErrorKind::ArgumentConflict, "{argv:?}");
        }
    }

    #[test]
    fn one_scope_flag_parses() {
        for argv in [
            vec![],
            vec!["--ver", "02.51"],
            vec!["--updates"],
            vec!["--all"],
        ] {
            assert!(uninstall(&argv).is_ok(), "{argv:?}");
        }
    }
}

mod store_read_globals_tests {
    use super::*;

    fn refusal(argv: &[&str]) -> Option<String> {
        let mut full = vec!["cellgov"];
        full.extend_from_slice(argv);
        global_refusal(&Cli::try_parse_from(full).expect("the invocation parses"))
    }

    #[test]
    fn status_takes_quiet_because_it_drops_its_next_step_hint() {
        assert_eq!(refusal(&["--quiet", "status"]), None);
    }

    #[test]
    fn every_read_verb_takes_a_format() {
        for argv in [
            vec!["--format", "json", "status"],
            vec!["--format", "json", "firmware", "list"],
            vec!["--format", "json", "firmware", "show", "4.91"],
            vec!["--format", "json", "firmware", "verify", "4.91"],
            vec!["--format", "json", "title", "list"],
            vec!["--format", "json", "title", "show", "NPAA00001"],
            vec!["--format", "json", "title", "verify", "NPAA00001"],
        ] {
            assert_eq!(refusal(&argv), None, "{argv:?}");
        }
    }

    #[test]
    fn a_removal_renders_no_report_so_it_refuses_a_format() {
        for argv in [
            vec!["--format", "json", "title", "uninstall", "NPAA00001"],
            vec!["--format", "json", "firmware", "uninstall", "4.91"],
        ] {
            let said = refusal(&argv).unwrap_or_else(|| panic!("{argv:?} accepted --format"));
            assert!(said.starts_with("--format"), "{said}");
        }
    }
}
