use super::artifact::{persist_finding, run_replay, run_replay_with};
use super::campaign::{
    drive, plan_campaign, reduce_retained_finding, run_workers, worker_shard_index, CampaignRun,
    FuzzEngine,
};
use super::entry::{run, run_inner, run_inner_with_render, TEST_RENDER};
use super::*;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use cellgov_terminal::caps::RenderFlags;
use cellgov_terminal::progress::ProgressSink;

use cellgov_fuzz::artifact::{
    ArtifactError, ArtifactReference, ArtifactReplayError, FuzzFindingArtifact,
};
use cellgov_fuzz::raw_decode::{RawDecodeArtifact, RawDecodeStatus};
use cellgov_fuzz::report::Finding;
use cellgov_fuzz::{ppu, CampaignSchedule, CaseRange, FuzzTarget, GenerationStrategy};

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::parse::{
    try_parse, Command, DevCommand, FuzzArgs, FuzzCommand, FuzzRawArgs, FuzzRawDecoder,
    FuzzReduction, FuzzReductionPolicy, FuzzReplayArgs,
};

fn parse(argv: &[&str]) -> Result<FuzzArgs, clap::Error> {
    let mut args = vec!["cellgov", "dev", "fuzz"];
    args.extend_from_slice(argv);
    let cli = try_parse(&args.iter().map(ToString::to_string).collect::<Vec<_>>())?;
    let Command::Dev(DevCommand::Fuzz(fuzz)) = cli.command else {
        panic!("fuzz must remain a dev command");
    };
    Ok(fuzz)
}

#[test]
fn every_fuzz_mode_is_a_declarative_dev_subcommand() {
    for mode in [
        "ppu-instruction",
        "ppu-sequence",
        "spu-instruction",
        "spu-sequence",
        "semantic",
    ] {
        let parsed = parse(&[mode]).expect("mode must parse");
        assert!(
            matches!(
                (mode, parsed.command),
                ("ppu-instruction", FuzzCommand::PpuInstruction(_))
                    | ("ppu-sequence", FuzzCommand::PpuSequence(_))
                    | ("spu-instruction", FuzzCommand::SpuInstruction(_))
                    | ("spu-sequence", FuzzCommand::SpuSequence(_))
                    | ("semantic", FuzzCommand::Semantic(_))
            ),
            "{mode} selected the wrong engine"
        );
    }
    assert!(matches!(
        parse(&["raw", "ppu", "--count", "1"])
            .expect("PPU raw mode")
            .command,
        FuzzCommand::Raw(FuzzRawArgs {
            decoder: FuzzRawDecoder::Ppu,
            ..
        })
    ));
    assert!(matches!(
        parse(&["raw", "spu", "--full", "--shard", "1", "--shards", "4"])
            .expect("SPU raw mode")
            .command,
        FuzzCommand::Raw(FuzzRawArgs {
            decoder: FuzzRawDecoder::Spu,
            ..
        })
    ));
    assert!(matches!(
        parse(&["replay", "--artifact", "finding.json"])
            .expect("artifact mode")
            .command,
        FuzzCommand::Replay(FuzzReplayArgs { reduced: false, .. })
    ));
    assert!(matches!(
        parse(&["replay", "--artifact", "finding.json", "--reduced"])
            .expect("reduced artifact mode")
            .command,
        FuzzCommand::Replay(FuzzReplayArgs { reduced: true, .. })
    ));
}

#[test]
fn quiet_progress_campaign_parses_and_dispatches() {
    let cli = try_parse(
        &[
            "cellgov",
            "--quiet",
            "dev",
            "fuzz",
            "ppu-instruction",
            "--progress",
            "--count",
            "1",
            "--workers",
            "1",
        ]
        .map(str::to_string),
    )
    .expect("quiet campaign must parse");
    assert!(cli.globals.quiet);
    let quiet = cli.globals.quiet;
    let Command::Dev(DevCommand::Fuzz(parsed)) = cli.command else {
        panic!("quiet campaign must dispatch to fuzz")
    };
    let FuzzCommand::PpuInstruction(ref args) = parsed.command else {
        panic!("quiet campaign must select PPU instructions")
    };
    assert!(args.progress);
    assert_eq!(
        run_inner_with_render(
            &parsed,
            RenderFlags {
                quiet,
                ..TEST_RENDER
            }
        )
        .expect("quiet campaign must run"),
        CommandExitCode::SUCCESS
    );
}

#[derive(Default)]
struct RecordingSink {
    totals: Mutex<Vec<(usize, u64)>>,
    advanced: AtomicU64,
    items: Mutex<Vec<String>>,
}

impl ProgressSink for RecordingSink {
    fn phase(&self, _code: u8) {}
    fn totals(&self, items: usize, amount: u64) {
        self.totals.lock().expect("totals").push((items, amount));
    }
    fn preset_done(&self, _amount: u64) {}
    fn item_started(&self, name: &str) {
        self.items.lock().expect("items").push(name.to_owned());
    }
    fn advanced(&self, delta: u64) {
        self.advanced.fetch_add(delta, Ordering::Relaxed);
    }
    fn item_finished(&self) {}
    fn finished(&self) {}
}

fn drive_recorded(label: &str, argv: &[&str]) -> (CampaignRun, RecordingSink) {
    let scratch = cellgov_testkit::scratch::scratch_labeled(label);
    let artifacts = scratch.join("findings");
    let artifacts = artifacts.to_str().expect("scratch path is UTF-8");
    let mut full = vec![
        "ppu-instruction",
        "--workers",
        "1",
        "--artifacts-dir",
        artifacts,
    ];
    full.extend_from_slice(argv);
    let parsed = parse(&full).expect("campaign must parse");
    let FuzzCommand::PpuInstruction(args) = &parsed.command else {
        panic!("wrong mode")
    };
    let plan = plan_campaign(args, FuzzEngine::PpuInstruction).expect("plan");
    let mut state = CampaignRun::default();
    let sink = RecordingSink::default();
    drive(&plan, &mut state, &sink).expect("drive");
    assert_eq!(state.offset(), plan.limit());
    (state, sink)
}

#[test]
fn the_batch_loop_declares_the_range_as_its_denominator_and_advances_to_it() {
    let (_, sink) = drive_recorded("fuzz_progress_range", &["--count", "150"]);
    assert_eq!(*sink.totals.lock().expect("totals"), vec![(0, 150)]);
    assert_eq!(sink.advanced.load(Ordering::Relaxed), 150);
    // Three batches of at most 64: the item names walk the range.
    let items = sink.items.lock().expect("items");
    assert_eq!(items.len(), 3);
    assert!(items[0].starts_with("cases 0..=63"), "{}", items[0]);
    assert!(items[1].starts_with("cases 64..=127"), "{}", items[1]);
    assert!(items[2].starts_with("cases 128..=149"), "{}", items[2]);
}

#[test]
fn a_failure_line_waits_for_an_in_place_bar_and_prints_at_once_otherwise() {
    let held = CampaignRun::report_under(true, "fuzz: held");
    assert_eq!(held.held(), ["fuzz: held".to_owned()]);
    let printed = CampaignRun::report_under(false, "fuzz: printed");
    assert!(printed.held().is_empty());
}

#[test]
fn the_batch_loop_measures_a_cancelled_range_against_its_stop_offset() {
    let (_, sink) = drive_recorded(
        "fuzz_progress_cancelled",
        &["--count", "150", "--cancel-after", "70"],
    );
    assert_eq!(*sink.totals.lock().expect("totals"), vec![(0, 70)]);
    assert_eq!(sink.advanced.load(Ordering::Relaxed), 70);
    assert_eq!(sink.items.lock().expect("items").len(), 2);
}

#[test]
fn campaign_parser_keeps_replay_and_selection_fields_typed() {
    let parsed = parse(&[
        "spu-sequence",
        "--campaign-version",
        "4",
        "--seed",
        "77",
        "--first",
        "12",
        "--count",
        "30",
        "--workers",
        "2",
        "--shard",
        "1",
        "--shards",
        "3",
        "--deadline-ms",
        "300",
        "--progress",
        "--finding-limit",
        "9",
        "--sequence-words",
        "8",
        "--strategy",
        "structured",
        "--check",
        "all",
        "--reduction",
        "on-finding",
        "--reduction-policy",
        "greedy",
        "--reduction-budget",
        "16",
    ])
    .expect("typed campaign");
    let FuzzCommand::SpuSequence(args) = parsed.command else {
        panic!("wrong mode")
    };
    assert!(matches!(
        (args.reduction, args.reduction_policy, args.reduction_budget),
        (FuzzReduction::OnFinding, FuzzReductionPolicy::Greedy, 16)
    ));
    assert_eq!(
        (args.campaign_version, args.seed, args.first, args.count),
        (4, 77, 12, 30)
    );
    assert_eq!((args.workers, args.shard, args.shards), (Some(2), 1, 3));
    assert_eq!(
        (
            args.deadline_ms,
            args.progress,
            args.finding_limit,
            args.sequence_words
        ),
        (Some(300), true, 9, Some(8))
    );
}

#[test]
fn raw_scope_and_replay_conflicts_fail_during_parse() {
    assert_eq!(
        parse(&["raw", "ppu"]).expect_err("scope required").kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );
    assert_eq!(
        parse(&["raw", "ppu", "--full", "--count", "3"])
            .expect_err("conflict")
            .kind(),
        clap::error::ErrorKind::ArgumentConflict
    );
    assert_eq!(
        parse(&["ppu-instruction", "--replay-case", "7", "--count", "2"])
            .expect_err("conflict")
            .kind(),
        clap::error::ErrorKind::ArgumentConflict
    );
    assert_eq!(
        parse(&["spu-instruction", "--stratgey", "raw-words"])
            .expect_err("typo")
            .kind(),
        clap::error::ErrorKind::UnknownArgument
    );
}

#[test]
fn host_validation_rejects_unsupported_and_invalid_options() {
    let unsupported = parse(&["ppu-instruction", "--check", "paths"]).expect("typed selection");
    assert!(matches!(
        run_inner(&unsupported),
        Err(FuzzCliError::CheckUnavailable)
    ));
    let raw_reduction = parse(&["raw", "spu", "--count", "1", "--reduction", "on-finding"])
        .expect("typed selection");
    assert!(matches!(
        run_inner(&raw_reduction),
        Err(FuzzCliError::ReductionUnavailable)
    ));
    for args in [
        vec!["ppu-instruction", "--workers", "0"],
        vec!["ppu-instruction", "--deadline-ms", "0"],
        vec!["ppu-instruction", "--finding-limit", "0"],
        vec![
            "ppu-instruction",
            "--reduction",
            "on-finding",
            "--reduction-budget",
            "0",
        ],
        vec!["ppu-instruction", "--sequence-words", "8"],
        vec!["ppu-instruction", "--count", "0"],
        vec!["ppu-instruction", "--shard", "2", "--shards", "2"],
    ] {
        let parsed = parse(&args).expect("typed flags parse");
        assert!(run_inner(&parsed).is_err(), "{args:?}");
    }
    let version = parse(&[
        "ppu-instruction",
        "--campaign-version",
        "999",
        "--count",
        "1",
        "--workers",
        "1",
    ])
    .expect("version parses");
    let error = run(&version).expect_err("unsupported generator version");
    assert_eq!(
        error.code().expect("status").value(),
        super::super::exit_codes::USAGE as u8
    );
    let sequence = parse(&[
        "spu-sequence",
        "--sequence-words",
        "0",
        "--count",
        "1",
        "--workers",
        "1",
    ])
    .expect("word count parses");
    let error = run(&sequence).expect_err("empty sequence");
    assert_eq!(
        error.code().expect("status").value(),
        super::super::exit_codes::USAGE as u8
    );
}

#[test]
fn selected_independent_references_are_replayed_and_checked() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/cellgov_fuzz/tests/fixtures");
    let ppu = root.join("ppu_reference/li_r3_7_v1.json");
    let spu = root.join("spu_reference/rotqbyi_12_v1.json");
    let parsed = parse(&["ppu-instruction", "--reference", "vector.json"])
        .expect("reference selector parses");
    let FuzzCommand::PpuInstruction(args) = parsed.command else {
        panic!("PPU mode")
    };
    assert_eq!(
        args.reference,
        Some(std::path::PathBuf::from("vector.json"))
    );
    let mut ppu_run =
        parse(&["ppu-instruction", "--replay-case", "0", "--workers", "1"]).expect("PPU campaign");
    let FuzzCommand::PpuInstruction(ref mut args) = ppu_run.command else {
        panic!("PPU mode")
    };
    args.reference = Some(ppu.clone());
    assert_eq!(
        run_inner(&ppu_run).expect("PPU reference"),
        CommandExitCode::SUCCESS
    );
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_reference_mismatch");
    let mismatched = scratch.join("ppu.json");
    let mut altered: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&ppu).expect("PPU reference fixture"))
            .expect("reference JSON");
    altered["expected"]["state"]["gpr"]["value"][3] = serde_json::json!(8);
    std::fs::write(
        &mismatched,
        serde_json::to_vec(&altered).expect("encode JSON"),
    )
    .expect("write altered reference");
    let FuzzCommand::PpuInstruction(ref mut args) = ppu_run.command else {
        panic!("PPU mode")
    };
    args.reference = Some(mismatched);
    assert!(matches!(
        run_inner(&ppu_run),
        Err(FuzzCliError::ReferenceMismatch)
    ));
    let FuzzCommand::PpuInstruction(ref mut args) = ppu_run.command else {
        panic!("PPU mode")
    };
    args.reference = Some(spu.clone());
    assert!(matches!(
        run_inner(&ppu_run),
        Err(FuzzCliError::PpuReference(_))
    ));

    let mut spu_run = parse(&[
        "spu-instruction",
        "--replay-case",
        "0",
        "--seed",
        "11",
        "--workers",
        "1",
    ])
    .expect("SPU campaign");
    let FuzzCommand::SpuInstruction(ref mut args) = spu_run.command else {
        panic!("SPU mode")
    };
    args.reference = Some(spu.clone());
    assert_eq!(
        run_inner(&spu_run).expect("SPU reference"),
        CommandExitCode::SUCCESS
    );
    let mismatched = scratch.join("spu.json");
    let mut altered: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&spu).expect("SPU reference fixture"))
            .expect("reference JSON");
    altered["expected"]["regs_hex"]["value"]["4"] =
        serde_json::json!("000102030405060708090a0b0c0d0e0f");
    std::fs::write(
        &mismatched,
        serde_json::to_vec(&altered).expect("encode JSON"),
    )
    .expect("write altered reference");
    let FuzzCommand::SpuInstruction(ref mut args) = spu_run.command else {
        panic!("SPU mode")
    };
    args.reference = Some(mismatched);
    assert!(matches!(
        run_inner(&spu_run),
        Err(FuzzCliError::ReferenceMismatch)
    ));
}

#[test]
fn parser_suggests_the_known_option_after_a_typo() {
    let error = parse(&["ppu-instruction", "--stratgey", "raw-words"]).expect_err("unknown option");
    assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    assert!(error.to_string().contains("--strategy"));
}

#[test]
fn short_campaigns_and_semantic_enumeration_reach_the_library() {
    let campaign = parse(&[
        "ppu-instruction",
        "--count",
        "4",
        "--workers",
        "2",
        "--strategy",
        "structured",
    ])
    .expect("campaign parses");
    assert!(matches!(run_inner(&campaign), Ok(CommandExitCode::SUCCESS)));
    let reducing = parse(&[
        "spu-sequence",
        "--count",
        "4",
        "--workers",
        "1",
        "--sequence-words",
        "4",
        "--reduction",
        "on-finding",
        "--reduction-policy",
        "greedy",
    ])
    .expect("reducing campaign parses");
    assert!(matches!(run_inner(&reducing), Ok(CommandExitCode::SUCCESS)));
    let semantic = parse(&["semantic", "both"]).expect("semantic parses");
    assert!(matches!(run_inner(&semantic), Ok(CommandExitCode::SUCCESS)));
}

#[test]
fn a_retained_finding_that_stops_reproducing_keeps_its_original_case() {
    use cellgov_fuzz::reduce::{ReductionPolicy, ReductionRequest, DEFAULT_REDUCTION_BUDGET};
    use cellgov_fuzz::report::{CheckIdentity, DivergenceClass, FindingKind, SemanticFingerprint};
    use cellgov_fuzz::{FuzzConfig, ReductionError, ReductionOutcome};
    let artifact = synthetic_finding_artifact();
    let finding = Finding {
        fingerprint: SemanticFingerprint {
            target: FuzzTarget::PpuInstruction,
            instruction_kind: None,
            check: CheckIdentity::LegalOutcome,
            divergence: DivergenceClass::Outcome,
            outcome: None,
            effect: None,
        },
        kind: FindingKind::IllegalOutcome,
        replay: artifact.original.replay,
        original_words: artifact.original.words.clone(),
        observation: None,
        reduction: ReductionOutcome::NotAttempted,
        panic_payload: None,
    };
    let outcome = reduce_retained_finding(
        FuzzConfig::default(),
        &finding,
        ReductionRequest {
            policy: ReductionPolicy::Deterministic,
            budget: DEFAULT_REDUCTION_BUDGET,
        },
    );
    assert_eq!(
        outcome,
        ReductionOutcome::Failed(ReductionError::OriginalNotReproduced)
    );
}

#[test]
fn a_reduced_rerun_of_a_stored_finding_is_refused_with_its_reduced_case() {
    use cellgov_fuzz::artifact::ArtifactReduction;
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_artifact_reduced_rerun");
    let path = scratch.join("finding.json");
    let unreduced = synthetic_finding_artifact();
    persist_finding(&path, unreduced.clone()).expect("first write");
    let mut reduced = unreduced.clone();
    reduced.reduction = ArtifactReduction::Reduced {
        words: vec![0x3860_0006],
    };
    reduced.validate().expect("reduced artifact is complete");
    let error =
        persist_finding(&path, reduced.clone()).expect_err("the reduced case must not vanish");
    assert!(matches!(
        error,
        FuzzCliError::ArtifactReductionNotStored {
            stored: ArtifactReduction::NotAttempted,
            artifact: retained,
            ..
        } if retained.reduction == reduced.reduction
    ));
    persist_finding(&path, unreduced.clone()).expect("an unreduced rerun is idempotent");

    let reduced_path = scratch.join("reduced.json");
    persist_finding(&reduced_path, reduced.clone()).expect("reduced first write");
    persist_finding(&reduced_path, reduced).expect("an identical reduced rerun is idempotent");
    persist_finding(&reduced_path, unreduced)
        .expect("an unreduced rerun leaves the reduced file standing");
}

#[test]
fn raw_replay_with_no_decoded_case_does_not_report_clean_completion() {
    let config = cellgov_fuzz::FuzzConfig {
        seed: 38,
        strategy: GenerationStrategy::RawWords,
        schedule: CampaignSchedule {
            cases: CaseRange { first: 0, count: 1 },
            ..CampaignSchedule::default()
        },
        ..cellgov_fuzz::FuzzConfig::default()
    };
    let library = ppu::run_instructions(config);
    assert_eq!(library.report.cases, 1);
    assert_eq!(library.report.eligible_cases, 0);
    assert!(library.report.finding_counts.is_empty());

    let parsed = parse(&[
        "ppu-instruction",
        "--strategy",
        "raw-words",
        "--seed",
        "38",
        "--count",
        "1",
        "--workers",
        "1",
    ])
    .expect("raw replay parses");
    assert!(matches!(
        run_inner(&parsed),
        Err(FuzzCliError::NoEligibleCases {
            cases: 1,
            decoded: 0,
            unsupported: 0,
            undefined: 0
        })
    ));
    assert_eq!(
        run(&parsed)
            .expect_err("empty validation must fail")
            .code()
            .expect("failed status")
            .value(),
        u8::try_from(super::outcome::EXIT_NO_ELIGIBLE_CASES).expect("small code")
    );
}

#[test]
fn raw_spu_replays_report_unsupported_and_undefined_cases() {
    for (seed, unsupported, undefined) in [
        ("11853982799468969514", 0, 1),
        ("2090528820835688248", 1, 0),
    ] {
        let parsed = parse(&[
            "spu-instruction",
            "--strategy",
            "raw-words",
            "--seed",
            seed,
            "--count",
            "1",
            "--workers",
            "1",
        ])
        .expect("raw SPU replay parses");
        assert!(matches!(
            run_inner(&parsed),
            Err(FuzzCliError::NoEligibleCases {
                cases: 1,
                decoded: 1,
                unsupported: actual_unsupported,
                undefined: actual_undefined
            }) if actual_unsupported == unsupported && actual_undefined == undefined
        ));
    }
}

#[test]
fn bounded_raw_scan_writes_a_versioned_replayable_result() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_raw_bounded");
    let output = scratch.join("raw.json");
    let mut parsed = parse(&[
        "raw",
        "spu",
        "--start",
        "0x0",
        "--count",
        "257",
        "--workers",
        "2",
        "--chunk-size",
        "17",
    ])
    .expect("raw parses");
    let FuzzCommand::Raw(ref mut args) = parsed.command else {
        panic!("raw mode")
    };
    args.output = Some(output.clone());
    assert_eq!(
        run_inner(&parsed).expect("raw scan"),
        CommandExitCode::SUCCESS
    );
    let json = std::fs::read_to_string(&output).expect("written artifact");
    let artifact = RawDecodeArtifact::parse_json(&json).expect("versioned result");
    assert_eq!(artifact.processed, 257);
    assert_eq!(artifact.accepted + artifact.refused + artifact.panics, 257);
    assert_eq!(artifact.word_at(256), Some(256));
}

#[test]
fn cancelled_raw_scan_keeps_an_explicit_partial_artifact() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_raw_cancelled");
    let output = scratch.join("raw.json");
    let mut parsed = parse(&[
        "raw",
        "spu",
        "--count",
        "10",
        "--cancel-after",
        "3",
        "--chunk-size",
        "4",
        "--workers",
        "2",
    ])
    .expect("bounded scan parses");
    let FuzzCommand::Raw(ref mut args) = parsed.command else {
        panic!("raw mode")
    };
    args.output = Some(output.clone());
    assert_eq!(
        run_inner(&parsed).expect("partial result").value(),
        u8::try_from(super::outcome::EXIT_CANCELLED).expect("small code")
    );
    let json = std::fs::read_to_string(output).expect("partial artifact");
    let artifact = RawDecodeArtifact::parse_json(&json).expect("valid cancellation");
    assert_eq!(artifact.status, RawDecodeStatus::Cancelled);
    assert_eq!(artifact.processed, 3);
}

#[test]
fn invalid_raw_settings_and_write_failures_have_typed_status() {
    let zero_chunk = parse(&["raw", "ppu", "--count", "1", "--chunk-size", "0"]).expect("parse");
    let usage = run(&zero_chunk).expect_err("invalid chunk");
    assert_eq!(
        usage.code().expect("status").value(),
        super::super::exit_codes::USAGE as u8
    );
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_raw_write_refusal");
    let output = scratch.join("missing-parent").join("raw.json");
    let mut parsed = parse(&["raw", "ppu", "--count", "1"]).expect("parse");
    let FuzzCommand::Raw(ref mut args) = parsed.command else {
        panic!("raw mode")
    };
    args.output = Some(output);
    assert!(matches!(
        run_inner(&parsed),
        Err(FuzzCliError::Write { .. })
    ));
    let limit = parse(&["raw", "ppu", "--count", "1", "--finding-limit", "1"]).expect("parse");
    assert!(matches!(
        run_inner(&limit),
        Err(FuzzCliError::RawFindingLimit)
    ));
}

#[test]
fn broken_stdout_retains_the_pipe_status() {
    let error = CommandError::from(FuzzCliError::Stdout(std::io::Error::from(
        std::io::ErrorKind::BrokenPipe,
    )));
    assert_eq!(
        error.code().expect("status").value(),
        super::super::exit_codes::BROKEN_PIPE as u8
    );
}

#[test]
fn a_panicking_worker_does_not_unwind_the_host_and_all_workers_join() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let completed = Arc::new(AtomicUsize::new(0));
    let work = (0..2)
        .map(|index| {
            let completed = Arc::clone(&completed);
            move || {
                if index == 0 {
                    panic!("seeded worker panic");
                }
                completed.fetch_add(1, Ordering::SeqCst);
                index
            }
        })
        .collect();
    assert!(matches!(run_workers(work), Err(FuzzCliError::WorkerPanic)));
    assert_eq!(completed.load(Ordering::SeqCst), 1);
}

#[test]
fn every_generated_engine_dispatches_with_replay_coordinates() {
    for mode in [
        "ppu-instruction",
        "ppu-sequence",
        "spu-instruction",
        "spu-sequence",
    ] {
        let parsed = parse(&[mode, "--replay-case", "0", "--seed", "11", "--workers", "1"])
            .expect("exact case parses");
        assert_eq!(
            run_inner(&parsed).expect("replayed case must run"),
            CommandExitCode::SUCCESS,
            "{mode}"
        );
    }
}

#[test]
fn host_workers_preserve_the_selected_shard_across_batches() {
    use cellgov_fuzz::{CampaignSchedule, CampaignShard, CaseRange};
    let first = 100u64;
    let count = 130u64;
    let mut observed = std::collections::BTreeSet::new();
    for offset in [0u64, 64, 128] {
        let batch = (count - offset).min(64);
        for worker in 0..3u32 {
            let index = worker_shard_index(1, 2, worker, 6, (offset % 6) as u32)
                .expect("valid worker partition");
            let schedule = CampaignSchedule {
                cases: CaseRange {
                    first: first + offset,
                    count: batch,
                },
                shard: CampaignShard { index, count: 6 },
                cancellation: None,
            };
            for case in schedule.case_indices().expect("bounded schedule") {
                assert!(observed.insert(case), "case {case} was assigned twice");
            }
        }
    }
    let expected = (first..first + count)
        .filter(|case| (case - first) % 2 == 1)
        .collect();
    assert_eq!(observed, expected);
}

pub(super) fn synthetic_finding_artifact() -> FuzzFindingArtifact {
    use cellgov_fuzz::artifact::{
        ArtifactCase, ArtifactCoverage, ArtifactFingerprint, ArtifactReduction, ArtifactStateSource,
    };
    use cellgov_fuzz::{FuzzConfig, FuzzTarget, GenerationStrategy, ReplayCoordinates};
    FuzzFindingArtifact {
        schema_version: cellgov_fuzz::artifact::FINDING_ARTIFACT_VERSION,
        campaign: FuzzConfig::default(),
        execution: cellgov_fuzz::artifact::ArtifactExecutionPolicy {
            workers: 1,
            deadline_ms: None,
            progress: false,
            check: cellgov_fuzz::artifact::ArtifactCheckSelection::All,
            reduction: cellgov_fuzz::artifact::ArtifactReductionRequest::None,
        },
        original: ArtifactCase {
            replay: ReplayCoordinates {
                campaign_version: cellgov_fuzz::CAMPAIGN_VERSION,
                target: FuzzTarget::PpuInstruction,
                strategy: GenerationStrategy::Structured,
                seed: 1,
                case_index: 0,
                sequence_words: 32,
            },
            words: vec![0x3860_0007],
            state_source: ArtifactStateSource::VersionedGenerator,
        },
        finding_kind: "IllegalOutcome".into(),
        fingerprint: ArtifactFingerprint {
            target: FuzzTarget::PpuInstruction,
            instruction_kind: None,
            check: "LegalOutcome".into(),
            divergence: "Outcome".into(),
            outcome: None,
            effect: None,
        },
        reference: ArtifactReference::Local,
        observation: None,
        coverage: ArtifactCoverage {
            cases: 1,
            decoded: 1,
            eligible: 1,
            unsupported: 0,
            undefined: 0,
            finding_counts: std::collections::BTreeMap::from([("IllegalOutcome".into(), 1)]),
        },
        reduction: ArtifactReduction::NotAttempted,
        panic_payload: None,
        replay_command: vec![
            "cellgov".into(),
            "dev".into(),
            "fuzz".into(),
            "replay".into(),
            "--artifact".into(),
            "finding.json".into(),
        ],
    }
}

#[test]
fn artifact_storage_keeps_existing_evidence_and_returns_the_original_on_failure() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_artifact_storage");
    let path = scratch.join("finding.json");
    let artifact = synthetic_finding_artifact();
    artifact.validate().expect("synthetic artifact is complete");
    persist_finding(&path, artifact.clone()).expect("first write");
    persist_finding(&path, artifact.clone()).expect("identical result is idempotent");
    let on_disk = std::fs::read_to_string(&path).expect("stored artifact");
    assert_eq!(
        FuzzFindingArtifact::parse_json(&on_disk).expect("versioned JSON"),
        artifact
    );

    let mut rerun = artifact.clone();
    rerun.execution.workers = 8;
    rerun.execution.progress = true;
    rerun.campaign.schedule.cases.count = 4;
    rerun.coverage.cases = 4;
    persist_finding(&path, rerun).expect("same finding under another host budget");
    assert_eq!(
        std::fs::read_to_string(&path).expect("first evidence kept"),
        on_disk
    );

    let mut conflicting = artifact.clone();
    conflicting.original.words[0] ^= 1;
    let collision = persist_finding(&path, conflicting.clone())
        .expect_err("different evidence cannot overwrite");
    assert!(
        matches!(collision,FuzzCliError::ArtifactCollision {artifact: retained,..}
        if retained.original.words == conflicting.original.words)
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("old file intact"),
        on_disk
    );

    let truncated = scratch.join("truncated.json");
    std::fs::write(&truncated, &on_disk.as_bytes()[..on_disk.len() / 2]).expect("partial file");
    assert!(matches!(
        persist_finding(&truncated, artifact.clone()),
        Err(FuzzCliError::ArtifactCollision { .. })
    ));

    let blocked = scratch.join("occupied");
    std::fs::write(&blocked, b"file").expect("blocking file");
    let error = persist_finding(&blocked.join("finding.json"), artifact.clone())
        .expect_err("a file cannot act as a directory");
    assert!(
        matches!(error,FuzzCliError::ArtifactWrite {artifact: retained,..}
        if retained.original.words == artifact.original.words)
    );
}

#[test]
fn artifact_replay_refuses_an_unreproduced_case_and_a_changed_schema() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_artifact_replay");
    let path = scratch.join("finding.json");
    let artifact = synthetic_finding_artifact();
    persist_finding(&path, artifact.clone()).expect("write artifact");
    let args = FuzzReplayArgs {
        artifact: path.clone(),
        reduced: false,
    };
    assert!(matches!(
        run_replay(&args),
        Err(FuzzCliError::ArtifactReplay(
            ArtifactReplayError::NotReproduced { case_index: 0 }
        ))
    ));
    let reduced = FuzzReplayArgs {
        artifact: path,
        reduced: true,
    };
    assert!(matches!(
        run_replay(&reduced),
        Err(FuzzCliError::ArtifactReplay(
            ArtifactReplayError::NoReducedCase
        ))
    ));
    let newer = FuzzReplayArgs {
        artifact: scratch.join("future.json"),
        reduced: false,
    };
    let mut changed = artifact;
    changed.schema_version += 1;
    persist_finding(&newer.artifact, changed).expect("write changed schema for refusal");
    assert!(matches!(
        run_replay(&newer),
        Err(FuzzCliError::Artifact(ArtifactError::Version { .. }))
    ));
}

#[test]
fn a_reproduced_finding_is_reported_as_a_failed_run() {
    use cellgov_fuzz::report::{CheckIdentity, DivergenceClass, FindingKind, SemanticFingerprint};
    use cellgov_fuzz::ReductionOutcome;
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_artifact_reproduced");
    let path = scratch.join("finding.json");
    let artifact = synthetic_finding_artifact();
    persist_finding(&path, artifact.clone()).expect("write artifact");
    let args = FuzzReplayArgs {
        artifact: path,
        reduced: false,
    };
    let finding = Finding {
        fingerprint: SemanticFingerprint {
            target: FuzzTarget::PpuInstruction,
            instruction_kind: None,
            check: CheckIdentity::LegalOutcome,
            divergence: DivergenceClass::Outcome,
            outcome: None,
            effect: None,
        },
        kind: FindingKind::IllegalOutcome,
        replay: artifact.original.replay,
        original_words: artifact.original.words.clone(),
        observation: None,
        reduction: ReductionOutcome::NotAttempted,
        panic_payload: None,
    };
    let code = run_replay_with(&args, |stored| {
        assert_eq!(stored, &artifact);
        Ok(finding)
    })
    .expect("the stored finding reproduces");
    assert_eq!(code, CommandExitCode::new(super::super::exit_codes::FAILED));
}

fn non_utf8_path() -> std::path::PathBuf {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        std::path::PathBuf::from(std::ffi::OsString::from_wide(&[0xD800]))
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        std::path::PathBuf::from(std::ffi::OsString::from_vec(vec![0xFF]))
    }
}

#[test]
fn a_non_utf8_artifacts_dir_is_refused_before_any_case_runs() {
    let mut parsed =
        parse(&["ppu-instruction", "--count", "1", "--workers", "1"]).expect("campaign parses");
    let FuzzCommand::PpuInstruction(ref mut args) = parsed.command else {
        panic!("PPU mode")
    };
    args.artifacts_dir = non_utf8_path();
    assert!(args.artifacts_dir.to_str().is_none());
    let error = run(&parsed).expect_err("artifact directory cannot be replayed");
    assert_eq!(
        error.code().expect("status").value(),
        super::super::exit_codes::USAGE as u8
    );
}
