use std::path::{Path, PathBuf};

use clap::Parser;

use super::entry::run;
use super::outcome::EXIT_VACUOUS;
use crate::cli::exit::CommandExitCode;
use crate::cli::exit_codes;
use crate::cli::parse::{FuzzArgs, FuzzCommand, PROMOTE_EXIT_CODES, SMOKE_EXIT_CODES};

#[derive(Parser)]
struct Harness {
    #[command(flatten)]
    fuzz: FuzzArgs,
}

fn parse(argv: &[&str]) -> Result<FuzzArgs, clap::Error> {
    let mut full = vec!["fuzz"];
    full.extend_from_slice(argv);
    Harness::try_parse_from(full).map(|harness| harness.fuzz)
}

fn path_arg(path: &Path) -> String {
    path.to_str().expect("scratch paths are UTF-8").to_owned()
}

/// The tracked regression directory of the fuzz crate.
fn regressions_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the CLI manifest dir is two levels under the workspace root")
        .join("crates")
        .join("cellgov_fuzz")
        .join("regressions")
}

fn smoke(
    artifacts: &Path,
    extra: &[&str],
) -> Result<CommandExitCode, crate::cli::exit::CommandError> {
    let artifacts = path_arg(artifacts);
    let mut argv = vec!["smoke", "--artifacts-dir", artifacts.as_str()];
    argv.extend_from_slice(extra);
    let parsed = parse(&argv).expect("smoke parses");
    assert!(matches!(parsed.command, FuzzCommand::Smoke(_)));
    run(&parsed)
}

#[test]
fn the_smoke_set_parses_its_typed_settings() {
    let parsed = parse(&[
        "smoke",
        "--artifacts-dir",
        "out",
        "--regressions",
        "regressions",
        "--reduction-budget",
        "32",
        "--progress",
    ])
    .expect("parses");
    let FuzzCommand::Smoke(args) = parsed.command else {
        panic!("smoke did not parse as the smoke command");
    };
    assert_eq!(args.artifacts_dir, PathBuf::from("out"));
    assert_eq!(args.regressions, Some(PathBuf::from("regressions")));
    assert_eq!(args.reduction_budget, 32);
    assert!(args.progress);
    let defaults = parse(&["smoke"]).expect("parses with defaults");
    let FuzzCommand::Smoke(args) = defaults.command else {
        panic!("smoke did not parse as the smoke command");
    };
    // The default sits two components under the workspace, in the build
    // output tree, so a clone carries no artifact it did not just store.
    assert_eq!(args.artifacts_dir.components().count(), 2);
    assert_eq!(
        args.artifacts_dir
            .file_name()
            .and_then(|name| name.to_str()),
        Some("fuzz-smoke")
    );
    assert_eq!(args.regressions, None);
    assert_eq!(
        args.reduction_budget,
        cellgov_fuzz::reduce::DEFAULT_REDUCTION_BUDGET
    );
}

#[test]
fn the_smoke_set_is_clean_against_the_tracked_regressions() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_smoke_tracked");
    let regressions = path_arg(&regressions_dir());
    let code = smoke(&scratch, &["--regressions", regressions.as_str()]).expect("the set runs");
    assert_eq!(code, CommandExitCode::SUCCESS);
}

#[test]
fn a_finding_no_regression_covers_fails_the_set_and_stores_its_minimized_artifact() {
    // The raw-word campaigns reach invalid load and store forms. A debug
    // build's PPU executor refuses those with a debug invariant; a release
    // build runs them silently. The promoted regressions name that profile,
    // so a run with no regressions is the one way to see both paths: the
    // finding path in a debug build, the clean path in a release build.
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_smoke_unpromoted");
    let code = smoke(&scratch, &[]).expect("the set runs");
    if cfg!(debug_assertions) {
        assert_eq!(code.value(), exit_codes::FAILED as u8);
        let mut stored = std::fs::read_dir(&*scratch)
            .expect("the artifacts directory exists")
            .map(|entry| entry.expect("entry").path())
            .collect::<Vec<_>>();
        stored.sort();
        assert!(!stored.is_empty(), "a finding stores its artifact");
        for path in stored {
            let artifact = cellgov_fuzz::artifact::FuzzFindingArtifact::parse_json(
                &std::fs::read_to_string(&path).expect("reads the artifact"),
            )
            .expect("the stored artifact validates");
            assert!(
                matches!(
                    artifact.reduction,
                    cellgov_fuzz::artifact::ArtifactReduction::Reduced { .. }
                        | cellgov_fuzz::artifact::ArtifactReduction::Irreducible
                ),
                "{}: the smoke set stores minimized findings only",
                path.display()
            );
            assert_eq!(artifact.finding_kind, "TargetPanic");
            // The stored replay names the directory as the caller spelled
            // it and the file after one forward slash, on either platform.
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("a stored artifact has a UTF-8 file name");
            assert_eq!(
                artifact.replay_command[5],
                format!("{}/{file_name}", path_arg(&scratch)),
                "{}: the replay path is portable text",
                path.display()
            );
        }
    } else {
        assert_eq!(code, CommandExitCode::SUCCESS);
        assert!(
            !scratch.exists()
                || std::fs::read_dir(&*scratch)
                    .expect("reads")
                    .next()
                    .is_none(),
            "a clean set stores nothing"
        );
    }
}

#[test]
fn a_regression_directory_that_cannot_stand_is_refused_before_any_campaign_runs() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_smoke_bad_regressions");
    let artifacts = scratch.join("artifacts");
    let regressions = scratch.join("regressions");
    std::fs::create_dir_all(&regressions).expect("creates");
    std::fs::write(regressions.join("manifest.json"), b"[]").expect("writes");
    let regressions = path_arg(&regressions);
    let error = smoke(&artifacts, &["--regressions", regressions.as_str()])
        .expect_err("a non-manifest is refused");
    assert_eq!(
        error.code().expect("status").value(),
        exit_codes::FAILED as u8
    );
    assert!(!artifacts.exists(), "no campaign ran");
    let error = smoke(&artifacts, &["--reduction-budget", "0"]).expect_err("zero budget");
    assert_eq!(
        error.code().expect("status").value(),
        exit_codes::USAGE as u8
    );
}

#[test]
fn promotion_copies_a_minimized_artifact_into_a_fresh_regression_directory() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_promote");
    let regressions = scratch.join("regressions");
    std::fs::create_dir_all(&regressions).expect("creates");
    std::fs::write(
        regressions.join("manifest.json"),
        b"{\"schema_version\":1,\"regressions\":[]}\n",
    )
    .expect("writes an empty manifest");
    // The tracked directory's first entry is a minimized artifact that
    // exists on every clone.
    let tracked = cellgov_fuzz::regression::load(&regressions_dir()).expect("tracked loads");
    let source = tracked.first().expect("a tracked regression exists");
    let artifact = path_arg(&source.path);
    let regressions_arg = path_arg(&regressions);
    let parsed = parse(&[
        "promote",
        "--artifact",
        artifact.as_str(),
        "--regressions",
        regressions_arg.as_str(),
        "--name",
        "copied",
        "--summary",
        "a copy of a tracked regression",
        "--profile",
        "release",
    ])
    .expect("promote parses");
    assert!(matches!(parsed.command, FuzzCommand::Promote(_)));
    assert_eq!(run(&parsed).expect("promotes"), CommandExitCode::SUCCESS);
    let copied = cellgov_fuzz::regression::load(&regressions).expect("loads");
    assert_eq!(copied.len(), 1);
    assert_eq!(copied[0].entry.name, "copied");
    assert_eq!(
        copied[0].entry.status,
        cellgov_fuzz::regression::RegressionStatus::Open
    );
    assert_eq!(
        copied[0].entry.profile,
        cellgov_fuzz::regression::RegressionProfile::Release
    );
    assert!(copied[0].artifact.describes_same_finding(&source.artifact));
    assert_eq!(copied[0].path, regressions.join("copied.json"));
    assert_eq!(
        copied[0].artifact.replay_command[5],
        format!("{}/copied.json", regressions_arg.replace('\\', "/")),
        "a promoted replay path is spelled with forward slashes on every host"
    );
    let error = run(&parsed).expect_err("the same name is refused");
    assert_eq!(
        error.code().expect("status").value(),
        exit_codes::FAILED as u8
    );
    let absent = parse(&[
        "promote",
        "--artifact",
        path_arg(&scratch.join("absent.json")).as_str(),
        "--regressions",
        regressions_arg.as_str(),
        "--name",
        "absent",
        "--summary",
        "nothing",
    ])
    .expect("parses");
    let error = run(&absent).expect_err("an absent artifact is refused");
    assert_eq!(
        error.code().expect("status").value(),
        exit_codes::FAILED as u8
    );
    assert_eq!(
        cellgov_fuzz::regression::load(&regressions)
            .expect("loads")
            .len(),
        1
    );
}

#[test]
fn the_smoke_outcome_ranks_its_causes_in_the_documented_order() {
    use super::outcome::SmokeOutcome;
    // Every cause at once resolves to the first in precedence; each row
    // then clears one cause and reaches the next.
    let ranked = [
        (
            (true, true, 1, 1, true),
            SmokeOutcome::EvidenceNotStored,
            super::outcome::EXIT_EVIDENCE_NOT_STORED,
        ),
        (
            (false, true, 1, 1, true),
            SmokeOutcome::HarnessFailure,
            super::outcome::EXIT_HARNESS_FAILURE,
        ),
        (
            (false, false, 1, 1, true),
            SmokeOutcome::ReductionFailed,
            super::outcome::EXIT_REDUCTION_FAILED,
        ),
        (
            (false, false, 0, 1, true),
            SmokeOutcome::Unpromoted,
            exit_codes::FAILED,
        ),
        (
            (false, false, 0, 0, true),
            SmokeOutcome::Vacuous,
            EXIT_VACUOUS,
        ),
        ((false, false, 0, 0, false), SmokeOutcome::Clean, 0),
    ];
    for ((not_stored, harness, reductions, unpromoted, vacuous), outcome, code) in ranked {
        let classified =
            SmokeOutcome::classify(not_stored, harness, reductions, unpromoted, vacuous);
        assert_eq!(classified, outcome);
        assert_eq!(
            classified.exit_code().value(),
            u8::try_from(code).expect("small code"),
            "{outcome:?}"
        );
    }
}

#[test]
fn the_help_names_every_promote_status() {
    for code in [exit_codes::FAILED, exit_codes::BROKEN_PIPE] {
        assert!(
            PROMOTE_EXIT_CODES
                .lines()
                .any(|line| line.trim_start().starts_with(&format!("{code} "))),
            "promote help does not name status {code}"
        );
    }
}

#[test]
fn the_help_names_every_smoke_status() {
    for code in [
        exit_codes::FAILED,
        super::outcome::EXIT_HARNESS_FAILURE,
        super::outcome::EXIT_EVIDENCE_NOT_STORED,
        super::outcome::EXIT_REDUCTION_FAILED,
        EXIT_VACUOUS,
        exit_codes::BROKEN_PIPE,
    ] {
        assert!(
            SMOKE_EXIT_CODES
                .lines()
                .any(|line| line.trim_start().starts_with(&format!("{code} "))),
            "smoke help does not name status {code}"
        );
    }
}
