use super::entry::{run, run_inner};
use super::outcome::{render_census_progress, render_census_summary, CensusSummary};
use super::FuzzCliError;

use std::path::PathBuf;

use cellgov_fuzz::decode_census::{ClassCounts, DecodeCensusArtifact, DecodeCensusError};

use crate::cli::exit::CommandExitCode;
use crate::cli::exit_codes;
use crate::cli::parse::{try_parse, Command, DevCommand, FuzzArgs, FuzzCommand};

fn parse(argv: &[&str]) -> Result<FuzzArgs, clap::Error> {
    let mut args = vec!["cellgov", "dev", "fuzz"];
    args.extend_from_slice(argv);
    let cli = try_parse(&args.iter().map(ToString::to_string).collect::<Vec<_>>())?;
    let Command::Dev(DevCommand::Fuzz(fuzz)) = cli.command else {
        panic!("fuzz must remain a dev command");
    };
    Ok(fuzz)
}

fn census_to(argv: &[&str], output: PathBuf) -> (FuzzArgs, PathBuf) {
    let mut parsed = parse(argv).expect("census parses");
    let FuzzCommand::Census(ref mut args) = parsed.command else {
        panic!("census mode")
    };
    args.output = Some(output.clone());
    (parsed, output)
}

fn read_artifact(path: &PathBuf) -> DecodeCensusArtifact {
    let json = std::fs::read_to_string(path).expect("written artifact");
    DecodeCensusArtifact::parse_json(&json).expect("versioned result")
}

#[test]
fn census_and_merge_parse_their_scope_flags() {
    let full = parse(&["census", "--full", "--shard", "3", "--shards", "16"]).expect("full");
    let FuzzCommand::Census(args) = full.command else {
        panic!("census mode")
    };
    assert!(args.full);
    assert_eq!((args.shard, args.shards), (Some(3), Some(16)));
    let bounded = parse(&["census", "--start", "0x60000000", "--count", "5"]).expect("bounded");
    let FuzzCommand::Census(args) = bounded.command else {
        panic!("census mode")
    };
    assert_eq!((args.start, args.count), (Some(0x6000_0000), Some(5)));
    assert!(parse(&["census"]).is_err(), "a scope is required");
    assert!(parse(&["census", "--full", "--count", "1"]).is_err());
    let merged = parse(&["census-merge", "a.json", "b.json", "--output", "c.json"]).expect("merge");
    let FuzzCommand::CensusMerge(args) = merged.command else {
        panic!("merge mode")
    };
    assert_eq!(
        args.inputs,
        vec![PathBuf::from("a.json"), PathBuf::from("b.json")]
    );
    assert_eq!(args.output, Some(PathBuf::from("c.json")));
    assert!(parse(&["census-merge"]).is_err(), "an input is required");
}

#[test]
fn a_bounded_census_writes_a_versioned_result_independent_of_the_worker_count() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_census_bounded");
    let (one_worker, one_path) = census_to(
        &[
            "census",
            "--start",
            "0x7c000000",
            "--count",
            "3000",
            "--workers",
            "1",
        ],
        scratch.join("one.json"),
    );
    let (three_workers, three_path) = census_to(
        &[
            "census",
            "--start",
            "0x7c000000",
            "--count",
            "3000",
            "--workers",
            "3",
        ],
        scratch.join("three.json"),
    );
    assert_eq!(
        run_inner(&one_worker).expect("census"),
        CommandExitCode::SUCCESS
    );
    assert_eq!(
        run_inner(&three_workers).expect("census"),
        CommandExitCode::SUCCESS
    );
    let one = read_artifact(&one_path);
    assert_eq!(one.domain.count, 3000);
    assert_eq!(one.domain.first, 0x7c00_0000);
    assert!(one.is_clean());
    assert!(one.classes.canonical > 0 && one.classes.not_recognized > 0);
    assert_eq!(read_artifact(&three_path), one);
    // An interval shorter than the worker count still covers every word once.
    let (short, short_path) = census_to(
        &[
            "census",
            "--start",
            "0xffffffe",
            "--count",
            "2",
            "--workers",
            "3",
        ],
        scratch.join("short.json"),
    );
    assert_eq!(run_inner(&short).expect("census"), CommandExitCode::SUCCESS);
    let short = read_artifact(&short_path);
    assert_eq!((short.domain.first, short.domain.count), (0x0fff_fffe, 2));
}

#[test]
fn merging_two_shards_reproduces_the_census_of_their_union() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_census_merge");
    let (whole, whole_path) = census_to(
        &["census", "--start", "0x38000000", "--count", "500"],
        scratch.join("whole.json"),
    );
    let (low, low_path) = census_to(
        &["census", "--start", "0x38000000", "--count", "200"],
        scratch.join("low.json"),
    );
    let (high, high_path) = census_to(
        &["census", "--start", "0x380000c8", "--count", "300"],
        scratch.join("high.json"),
    );
    for args in [&whole, &low, &high] {
        assert_eq!(run_inner(args).expect("census"), CommandExitCode::SUCCESS);
    }
    let merged_path = scratch.join("merged.json");
    let merged = parse(&[
        "census-merge",
        high_path.to_str().expect("utf-8 path"),
        low_path.to_str().expect("utf-8 path"),
        "--output",
        merged_path.to_str().expect("utf-8 path"),
    ])
    .expect("merge parses");
    assert_eq!(run_inner(&merged).expect("merge"), CommandExitCode::SUCCESS);
    assert_eq!(read_artifact(&merged_path), read_artifact(&whole_path));
    let gapped = parse(&[
        "census-merge",
        whole_path.to_str().expect("utf-8 path"),
        high_path.to_str().expect("utf-8 path"),
    ])
    .expect("merge parses");
    assert!(matches!(
        run_inner(&gapped),
        Err(FuzzCliError::Census(
            DecodeCensusError::Discontiguous { .. }
        ))
    ));
    let short_of_full = parse(&[
        "census-merge",
        "--full",
        low_path.to_str().expect("utf-8 path"),
        high_path.to_str().expect("utf-8 path"),
    ])
    .expect("merge parses");
    assert!(matches!(
        run_inner(&short_of_full),
        Err(FuzzCliError::CensusIncomplete {
            first: 0x3800_0000,
            count: 500
        })
    ));
    let unreadable = parse(&["census-merge", "missing-census.json"]).expect("merge parses");
    assert!(matches!(
        run_inner(&unreadable),
        Err(FuzzCliError::CensusRead { .. })
    ));
}

#[test]
fn invalid_census_settings_and_write_failures_have_typed_status() {
    assert!(parse(&["census", "--full", "--start", "0x1"]).is_err());
    assert!(parse(&["census", "--count", "1", "--shard", "0"]).is_err());
    let mut mixed = parse(&["census", "--full"]).expect("parse");
    let FuzzCommand::Census(ref mut args) = mixed.command else {
        panic!("census mode")
    };
    args.start = Some(1);
    let usage = run(&mixed).expect_err("start is bounded-only");
    assert_eq!(
        usage.code().expect("status").value(),
        exit_codes::USAGE as u8
    );
    let mut sharded = parse(&["census", "--count", "1"]).expect("parse");
    let FuzzCommand::Census(ref mut args) = sharded.command else {
        panic!("census mode")
    };
    args.shard = Some(0);
    assert!(matches!(
        run_inner(&sharded),
        Err(FuzzCliError::Invalid("shard is full-only"))
    ));
    let empty = parse(&["census", "--count", "0"]).expect("parse");
    assert!(matches!(
        run_inner(&empty),
        Err(FuzzCliError::Census(DecodeCensusError::Domain(_)))
    ));
    let usage = run(&empty).expect_err("an empty interval is refused");
    assert_eq!(
        usage.code().expect("status").value(),
        exit_codes::USAGE as u8
    );
    let bad_shard = parse(&["census", "--full", "--shard", "16", "--shards", "16"]).expect("parse");
    assert!(matches!(
        run_inner(&bad_shard),
        Err(FuzzCliError::Census(DecodeCensusError::Domain(_)))
    ));
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_census_write_refusal");
    let (parsed, _) = census_to(
        &["census", "--count", "1"],
        scratch.join("missing-parent").join("census.json"),
    );
    assert!(matches!(
        run_inner(&parsed),
        Err(FuzzCliError::Write { .. })
    ));
}

#[test]
fn census_lines_render_from_their_records_and_a_finding_fails_the_command() {
    assert_eq!(render_census_progress(7, 10), "fuzz census: 7 of 10 words");
    let clean = CensusSummary {
        command: "fuzz census",
        words: 10,
        classes: ClassCounts {
            canonical: 3,
            reserved_bits: 2,
            alias: 1,
            round_trip_failures: 0,
            arm_unimplemented: 1,
            arm_unlisted: 0,
            not_recognized: 3,
            panics: 0,
        },
        primary_zero_decoded: 0,
        findings: 0,
        elapsed_seconds: 4,
        output: Some(PathBuf::from("census.json")),
    };
    assert_eq!(
        render_census_summary(&clean),
        "fuzz census: 10 words in 4s; canonical=3 reserved_bits=2 alias=1 round_trip_failures=0 \
         arm_unimplemented=1 arm_unlisted=0 not_recognized=3 panics=0 primary_zero_decoded=0 \
         findings=0 -> census.json\n"
    );
    assert_eq!(clean.exit_code(), CommandExitCode::SUCCESS);
    let mut failed = clean.clone();
    failed.command = "fuzz census-merge";
    failed.classes.round_trip_failures = 1;
    failed.classes.canonical = 2;
    failed.findings = 1;
    failed.output = None;
    assert_eq!(
        render_census_summary(&failed),
        "fuzz census-merge: 10 words in 4s; canonical=2 reserved_bits=2 alias=1 \
         round_trip_failures=1 arm_unimplemented=1 arm_unlisted=0 not_recognized=3 panics=0 \
         primary_zero_decoded=0 findings=1\n"
    );
    assert_eq!(failed.exit_code(), CommandExitCode::new(exit_codes::FAILED));
}
