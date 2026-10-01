//! `cellgov ps3`: each verb's flags against the runner's table, the
//! command each line maps to against the runner's own parse, the clap
//! pairings, and the status each runner failure maps to.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::PathBuf;

use clap::Parser as _;
use strum::VariantArray as _;

use super::*;
use crate::cli::parse::{Cli, Command, PS3_EXIT_CODES};

/// `cellgov ps3 <words>`, parsed.
fn parsed(words: &[&str]) -> Result<Ps3Command, clap::Error> {
    let mut argv = vec!["cellgov", "ps3"];
    argv.extend_from_slice(words);
    match Cli::try_parse_from(argv)?.command {
        Command::Ps3(command) => Ok(command),
        other => panic!("{other:?}"),
    }
}

/// The long flags the `ps3 <verb>` subcommand declares itself.
fn clap_flags(verb: &str) -> BTreeSet<String> {
    let tree = crate::cli::reference::command_tree();
    let command = tree
        .find_subcommand("ps3")
        .and_then(|ps3| ps3.find_subcommand(verb))
        .unwrap_or_else(|| panic!("cellgov ps3 has no {verb}"));
    command
        .get_arguments()
        .filter(|arg| !arg.is_global_set())
        .filter_map(|arg| arg.get_long())
        .filter(|long| *long != "help")
        .map(|long| format!("--{long}"))
        .collect()
}

#[test]
fn every_verb_takes_exactly_the_flags_the_runner_takes() {
    for verb in Verb::VARIANTS {
        let table: BTreeSet<String> = verb.flags().iter().map(|f| f.to_string()).collect();
        assert_eq!(clap_flags(verb.name()), table, "ps3 {}", verb.name());
    }
}

#[test]
fn the_noun_has_exactly_the_runners_verbs() {
    let tree = crate::cli::reference::command_tree();
    let ours: BTreeSet<&str> = tree
        .find_subcommand("ps3")
        .expect("cellgov ps3")
        .get_subcommands()
        .map(clap::Command::get_name)
        .filter(|name| *name != "help")
        .collect();
    let runner: BTreeSet<&str> = Verb::VARIANTS.iter().map(|v| v.name()).collect();
    assert_eq!(ours, runner);
}

/// One line per verb, each setting every flag that verb takes.
const LINES: &[&[&str]] = &[
    &[
        "status",
        "--host",
        "10.77.0.2",
        "--profile",
        "cech20-cex-493",
        "--profiles",
        "profiles.toml",
        "--model",
        "CECH-2001A",
        "--cfw",
        "EvilNAT 4.93 PEX",
        "--debugger",
        "none",
    ],
    &[
        "deploy",
        "--manifest",
        "m.toml",
        "--reclaim",
        "--debugger",
        "attached",
    ],
    &["run", "--manifest", "m.toml", "--poll-ms", "250"],
    &["fetch", "--manifest", "m.toml", "--out", "frame.bin"],
    &["cleanup", "--manifest", "m.toml", "--host", "ps3"],
    &[
        "capture",
        "--manifest",
        "m.toml",
        "--harness-revision",
        "0123abcd",
        "--out",
        "dir",
        "--poll-ms",
        "100",
        "--reclaim",
        "--keep-deployed",
        "--recapture",
        "--reason",
        "the SPU image changed",
    ],
    &[
        "convert",
        "--frame",
        "frame.bin",
        "--manifest",
        "m.toml",
        "--profile",
        "cech20-cex-493",
        "--profiles",
        "profiles.toml",
        "--out",
        "observation.json",
    ],
    &["unlock", "--host", "10.77.0.2"],
];

#[test]
fn each_line_maps_to_the_command_the_runner_parses_from_it() {
    for words in LINES {
        let ours = runner_command(&parsed(words).unwrap_or_else(|e| panic!("{words:?}: {e}")));
        let args: Vec<OsString> = words.iter().map(OsString::from).collect();
        let theirs = runner_ps3::cli::parse(&args).expect("the runner parses it");
        assert_eq!(ours, theirs, "{words:?}");
    }
    let covered: BTreeSet<&str> = LINES.iter().map(|words| words[0]).collect();
    assert_eq!(covered.len(), Verb::VARIANTS.len(), "a line per verb");
}

#[test]
fn a_line_the_runner_refuses_is_a_usage_error_here() {
    for words in [
        &[
            "capture",
            "--manifest",
            "m",
            "--harness-revision",
            "x",
            "--recapture",
        ][..],
        &[
            "capture",
            "--manifest",
            "m",
            "--harness-revision",
            "x",
            "--reason",
            "why",
        ][..],
        &["status", "--debugger", "maybe"][..],
        &["run", "--manifest", "m", "--poll-ms", "0"][..],
        &["unlock", "--manifest", "m"][..],
    ] {
        let error = parsed(words).expect_err("refused");
        assert_eq!(error.exit_code(), exit_codes::USAGE, "{words:?}: {error}");
        let args: Vec<OsString> = words.iter().map(OsString::from).collect();
        assert!(
            runner_ps3::cli::parse(&args).is_err(),
            "{words:?}: the runner refuses it too"
        );
    }
}

#[test]
fn each_runner_failure_maps_to_its_status() {
    let io = || std::io::Error::from(std::io::ErrorKind::NotFound);
    let cases: Vec<(RunnerPs3Error, i32)> = vec![
        (RunnerPs3Error::Usage("no --host".to_string()), 2),
        (
            RunnerPs3Error::from(ConsoleError::OperatorMissing {
                field: "model",
                flag: "--model",
            }),
            2,
        ),
        (
            RunnerPs3Error::LocalRead {
                path: PathBuf::from("frame.bin"),
                source: io(),
            },
            1,
        ),
        (
            RunnerPs3Error::from(
                cellgov_compare::manifest::parse_console("[test]").expect_err("no observe"),
            ),
            1,
        ),
        (
            RunnerPs3Error::from(
                cellgov_compare::hardware_capture::HardwareCaptureError::LooseFile(PathBuf::from(
                    "ps3/observation.json",
                )),
            ),
            1,
        ),
        (
            RunnerPs3Error::from(ConsoleProfileError::Io {
                path: PathBuf::from("console_profiles.toml"),
                source: io(),
            }),
            1,
        ),
        (
            RunnerPs3Error::from(ConsoleProfileError::from(
                toml::from_str::<toml::Table>("=").expect_err("bad toml"),
            )),
            1,
        ),
        (
            RunnerPs3Error::from(ConsoleProfileError::UnknownReference("x".to_string())),
            1,
        ),
        (
            RunnerPs3Error::from(ConsoleProfileError::NoModels("x".to_string())),
            1,
        ),
        (
            RunnerPs3Error::LocalWrite {
                path: PathBuf::from("out"),
                source: io(),
            },
            1,
        ),
        (RunnerPs3Error::HostClock, 1),
        (
            RunnerPs3Error::from(serde_json::from_str::<u8>("x").expect_err("not json")),
            1,
        ),
        (
            RunnerPs3Error::from(LeaseError::Io {
                path: PathBuf::from("lease"),
                source: io(),
            }),
            1,
        ),
        (
            RunnerPs3Error::Refused {
                reason: "a result file remains".to_string(),
                clear_with: "cellgov ps3 cleanup".to_string(),
            },
            50,
        ),
        (
            RunnerPs3Error::from(LeaseError::Held {
                path: PathBuf::from("lease"),
                host: "10.77.0.2".to_string(),
                holder: "pid=1 holder=x".to_string(),
                unlock_with: "cellgov ps3 unlock --host 10.77.0.2".to_string(),
            }),
            50,
        ),
        (
            RunnerPs3Error::from(ConsoleError::Contradiction {
                field: "model",
                page: "CECH-2501A".to_string(),
                operator: "CECH-2001A".to_string(),
            }),
            50,
        ),
        (
            RunnerPs3Error::from(ConsoleProfileError::Mismatch {
                profile: "cech20-cex-493".to_string(),
                mismatches: Vec::new(),
                satisfied: Vec::new(),
            }),
            50,
        ),
        (
            RunnerPs3Error::from(ConsoleProfileError::UnknownProfile {
                claimed: "cech25".to_string(),
                known: "cech20-cex-493".to_string(),
            }),
            50,
        ),
        (
            RunnerPs3Error::from(runner_ps3::transport::TransportError::ReplyTruncated),
            51,
        ),
        (
            RunnerPs3Error::from(ConsoleError::PageMissing { field: "firmware" }),
            51,
        ),
        (
            RunnerPs3Error::Timeout {
                result_path: "/dev_hdd0/tmp/cgov_x.bin".to_string(),
                timeout_ms: 30_000,
            },
            52,
        ),
        (RunnerPs3Error::Frame("magic absent".to_string()), 53),
        (
            RunnerPs3Error::Cleanup {
                remaining: "/dev_hdd0/game/CGOV00001".to_string(),
            },
            54,
        ),
    ];
    for (error, code) in &cases {
        assert_eq!(exit_code(error), *code, "{error}");
    }
}

#[test]
fn the_help_names_every_status_in_the_band() {
    for code in [
        EXIT_REFUSED,
        EXIT_TRANSPORT,
        EXIT_TIMEOUT,
        EXIT_FRAME,
        EXIT_CLEANUP,
    ] {
        assert!(
            PS3_EXIT_CODES.contains(&format!("\n  {code}  ")),
            "{code} is missing from:\n{PS3_EXIT_CODES}"
        );
    }
}
