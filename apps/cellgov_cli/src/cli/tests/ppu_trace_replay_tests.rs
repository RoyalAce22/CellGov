use std::path::{Path, PathBuf};

use cellgov_fuzz::ppu_trace::{console_vectors, replay_trace, PpuTraceOptions};
use cellgov_ppu::differential::rpcs3_capture::{
    read_trace_bytes, FORMAT_VERSION, HEADER_MAGIC, RECORD_MAGIC,
};
use cellgov_ppu::differential::{execute_state, PpuStateSnapshot};

use super::{render_human, render_json, replay, run, EXIT_NOTHING_REPLAYED};
use crate::cli::exit::CommandExitCode;
use crate::cli::exit_codes;
use crate::cli::parse::{try_parse, Command, DevCommand, OutputFormat, PpuTraceReplayArgs};

/// `li r3, 7`.
const LI: u32 = 0x3860_0007;

/// `divw r3, r4, r5`.
const DIVW: u32 = 0x7c64_2bd6;

/// `mftb r3`.
const MFTB: u32 = 0x7c6c_42e6;

fn put_state(out: &mut Vec<u8>, s: &PpuStateSnapshot) {
    for v in s.gpr.iter().chain(&s.fpr) {
        out.extend(v.to_le_bytes());
    }
    for v in &s.vr {
        out.extend(v.to_be_bytes());
    }
    out.extend(s.cr.to_le_bytes());
    for v in [s.lr, s.ctr, s.xer] {
        out.extend(v.to_le_bytes());
    }
    for v in [s.vrsave, s.fpscr, s.vscr, 0] {
        out.extend(v.to_le_bytes());
    }
    out.extend(0u64.to_le_bytes());
}

/// A `FORMAT_VERSION` trace of `(word, pre, post)` records with no storage access.
fn trace(records: &[(u32, PpuStateSnapshot, PpuStateSnapshot)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(HEADER_MAGIC.to_le_bytes());
    out.extend(FORMAT_VERSION.to_le_bytes());
    for (word, pre, post) in records {
        out.extend(RECORD_MAGIC.to_le_bytes());
        out.extend(0x1_0000u64.to_le_bytes());
        out.extend(word.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        put_state(&mut out, pre);
        put_state(&mut out, post);
        out.extend(0u64.to_le_bytes());
        out.extend(0u32.to_le_bytes());
    }
    out
}

/// A record whose RPCS3 end is CellGov's end with `change` applied.
fn step(
    word: u32,
    pre: PpuStateSnapshot,
    change: impl Fn(&mut PpuStateSnapshot),
) -> (u32, PpuStateSnapshot, PpuStateSnapshot) {
    let mut post = execute_state(word, &pre).expect("CellGov runs the word");
    change(&mut post);
    (word, pre, post)
}

fn divide_by_zero() -> PpuStateSnapshot {
    let mut pre = PpuStateSnapshot::zero();
    pre.gpr[4] = 7;
    pre
}

fn args(trace: &Path) -> PpuTraceReplayArgs {
    let argv = [
        "cellgov",
        "dev",
        "ppu-trace-replay",
        trace.to_str().expect("utf-8 path"),
        "--skip-context-dependent",
        "--max-failures",
        "3",
    ];
    let cli = try_parse(&argv.iter().map(ToString::to_string).collect::<Vec<_>>())
        .expect("ppu-trace-replay parses");
    let Command::Dev(DevCommand::PpuTraceReplay(args)) = cli.command else {
        panic!("ppu-trace-replay must be a dev command");
    };
    args
}

/// The committed console references.
fn references() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/cellgov_fuzz/tests/fixtures/ppu_reference")
}

fn args_with_references(trace: &Path, references: &Path) -> PpuTraceReplayArgs {
    let argv = [
        "cellgov",
        "dev",
        "ppu-trace-replay",
        trace.to_str().expect("utf-8 path"),
        "--references",
        references.to_str().expect("utf-8 path"),
    ];
    let cli = try_parse(&argv.iter().map(ToString::to_string).collect::<Vec<_>>())
        .expect("ppu-trace-replay parses");
    let Command::Dev(DevCommand::PpuTraceReplay(args)) = cli.command else {
        panic!("ppu-trace-replay must be a dev command");
    };
    args
}

#[test]
fn a_console_vector_under_references_turns_a_triage_exit_into_a_pass() {
    let dir = references();
    let console = console_vectors(&dir).expect("the committed references load");
    let vector = console
        .iter()
        .find(|v| v.file == "add.json" && v.name == "ppu_fixed_point_arith-add_p0")
        .expect("add.json holds the vector");
    let target = ((vector.word >> 21) & 0x1f) as usize;
    let scratch = cellgov_testkit::scratch::scratch_labeled("cli_ppu_trace_replay_refs");
    let path = scratch.join("divergence.bin");
    std::fs::write(
        &path,
        trace(&[step(vector.word, vector.start.clone(), |post| {
            post.gpr[target] ^= 1
        })]),
    )
    .expect("writes");
    assert_eq!(
        run(&args(&path), OutputFormat::Human).expect("runs"),
        CommandExitCode::new(exit_codes::FAILED)
    );
    let decided = args_with_references(&path, &dir);
    assert_eq!(decided.references.as_deref(), Some(dir.as_path()));
    let report = replay(&decided).expect("replays");
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].class.name(), "rpcs3-divergence");
    assert_eq!(
        run(&decided, OutputFormat::Human).expect("runs"),
        CommandExitCode::SUCCESS
    );
    let absent = scratch.join("no_references");
    let Err(refused) = run(&args_with_references(&path, &absent), OutputFormat::Human) else {
        panic!("a reference directory that does not exist is refused");
    };
    let refused = refused.to_string();
    assert!(
        refused.starts_with(&format!(
            "ppu-trace-replay: PPU trace reference read {}: ",
            absent.display()
        )),
        "{refused}"
    );
}

#[test]
fn a_trace_that_does_not_parse_is_refused_by_path() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("cli_ppu_trace_replay_bad");
    let path = scratch.join("bad.bin");
    std::fs::write(&path, [0u8; 8]).expect("writes");
    let Err(error) = replay(&args(&path)) else {
        panic!("a trace with a wrong magic is refused");
    };
    let cause = read_trace_bytes(&[0u8; 8]).expect_err("a wrong magic is refused");
    assert!(cause.to_string().contains("header magic"), "{cause}");
    assert_eq!(
        error.to_string(),
        format!("ppu-trace-replay: {}: {cause}", path.display())
    );
}

#[test]
fn the_flags_parse_into_the_arguments() {
    let args = args(Path::new("t.bin"));
    assert_eq!(args.trace, Path::new("t.bin"));
    assert!(args.skip_context_dependent);
    assert_eq!(args.max_failures, 3);
    assert_eq!(args.references, None);
}

#[test]
fn a_trace_needing_no_triage_exits_zero_and_one_needing_it_exits_one() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("cli_ppu_trace_replay");
    let dir: &Path = &scratch;
    let clean = dir.join("clean.bin");
    let open = step(DIVW, divide_by_zero(), |post| post.gpr[3] ^= 0xffff);
    std::fs::write(
        &clean,
        trace(&[step(LI, PpuStateSnapshot::zero(), |_| {}), open]),
    )
    .expect("writes");
    assert_eq!(
        run(&args(&clean), OutputFormat::Human).expect("runs"),
        CommandExitCode::SUCCESS
    );
    let wrong = dir.join("wrong.bin");
    std::fs::write(
        &wrong,
        trace(&[step(LI, PpuStateSnapshot::zero(), |post| post.gpr[3] = 8)]),
    )
    .expect("writes");
    assert_eq!(
        run(&args(&wrong), OutputFormat::Human).expect("runs"),
        CommandExitCode::new(exit_codes::FAILED)
    );
    let absent = dir.join("absent.bin");
    let Err(refused) = run(&args(&absent), OutputFormat::Human) else {
        panic!("a trace that does not exist is refused");
    };
    let refused = refused.to_string();
    assert!(
        refused.starts_with(&format!("ppu-trace-replay: {}: ", absent.display())),
        "{refused}"
    );
}

#[test]
fn the_skip_flag_reaches_the_replay() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("cli_ppu_trace_replay_skip");
    let path = scratch.join("mftb.bin");
    std::fs::write(
        &path,
        trace(&[step(MFTB, PpuStateSnapshot::zero(), |post| {
            post.gpr[3] = 1234
        })]),
    )
    .expect("writes");
    let mut args = args(&path);
    let skipped = replay(&args).expect("replays");
    assert_eq!((skipped.kinds[0].skipped, skipped.failures.len()), (1, 0));
    args.skip_context_dependent = false;
    let ran = replay(&args).expect("replays");
    assert_eq!((ran.kinds[0].skipped, ran.failures.len()), (0, 1));
}

#[test]
fn a_trace_with_no_replayed_record_does_not_exit_zero() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("cli_ppu_trace_replay_vacuous");
    let empty = scratch.join("header_only.bin");
    std::fs::write(&empty, trace(&[])).expect("writes");
    assert_eq!(
        run(&args(&empty), OutputFormat::Human).expect("runs"),
        CommandExitCode::new(EXIT_NOTHING_REPLAYED)
    );
    let report = replay(&args(&empty)).expect("replays");
    let json: serde_json::Value = serde_json::from_str(&render_json(&report, 3)).expect("JSON");
    assert_eq!(json["records"], 0);
    assert_eq!(json["control_registers"], "none");
    let skipped = scratch.join("all_skipped.bin");
    std::fs::write(
        &skipped,
        trace(&[step(MFTB, PpuStateSnapshot::zero(), |post| {
            post.gpr[3] = 1234
        })]),
    )
    .expect("writes");
    let mut args = args(&skipped);
    assert_eq!(
        run(&args, OutputFormat::Human).expect("runs"),
        CommandExitCode::new(EXIT_NOTHING_REPLAYED)
    );
    args.skip_context_dependent = false;
    assert_eq!(
        run(&args, OutputFormat::Human).expect("runs"),
        CommandExitCode::SUCCESS,
        "a replayed context-dependent failure needs no triage"
    );
}

#[test]
fn the_human_report_names_each_kind_class_and_field() {
    let bytes = trace(&[
        step(LI, PpuStateSnapshot::zero(), |_| {}),
        step(DIVW, divide_by_zero(), |post| post.gpr[3] ^= 0xffff),
        step(LI, PpuStateSnapshot::zero(), |post| post.gpr[3] = 8),
    ]);
    let records = read_trace_bytes(&bytes).expect("the trace reads");
    let report = replay_trace(&records, &[], PpuTraceOptions::default());
    let text = render_human(&report, 1);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[0],
        "ppu-trace-replay: 3 record(s); vrsave, fpscr and vscr compared"
    );
    assert!(lines.contains(&"kind Ordinary(Addi) seen=2 passed=1 failed=1 skipped=0"));
    assert!(lines.contains(&"kind Ordinary(Divw) seen=1 passed=0 failed=1 skipped=0"));
    assert!(lines.contains(
        &"failure record=1 pc=0x00010000 raw=0x7c642bd6 kind=Ordinary(Divw) \
          class=undefined (muldiv-undef) fields=[gpr3]"
    ));
    assert!(lines.contains(&"... 1 more failure(s) not listed"));
    assert!(
        !lines
            .iter()
            .any(|line| line.starts_with("failure record=2 ")),
        "a failure past the cap is not listed"
    );
    assert_eq!(
        lines.last(),
        Some(&"summary: 2 failure(s) [unclassified=1 undefined=1], 1 to triage")
    );
    let unreached = lines
        .iter()
        .find(|line| line.starts_with("unreached "))
        .expect("the unreached line");
    assert!(unreached.contains(" Ordinary(Subf)"), "{unreached}");
    assert!(!unreached.contains(" Ordinary(Addi)"), "{unreached}");
}

#[test]
fn the_json_report_counts_every_failure_and_lists_the_first() {
    let bytes = trace(&[
        step(DIVW, divide_by_zero(), |post| post.gpr[3] ^= 0xffff),
        step(LI, PpuStateSnapshot::zero(), |post| post.gpr[3] = 8),
    ]);
    let records = read_trace_bytes(&bytes).expect("the trace reads");
    let report = replay_trace(&records, &[], PpuTraceOptions::default());
    let json: serde_json::Value = serde_json::from_str(&render_json(&report, 1)).expect("JSON");
    assert_eq!(json["records"], 2);
    assert_eq!(json["control_registers"], "recorded");
    assert_eq!(json["failure_count"], 2);
    assert_eq!(json["triage_count"], 1);
    assert_eq!(json["failure_classes"]["undefined"], 1);
    assert_eq!(json["failure_classes"]["unclassified"], 1);
    assert_eq!(
        json["kinds"],
        serde_json::json!([
            {"kind": "Ordinary(Addi)", "seen": 1, "passed": 0, "failed": 1, "skipped": 0},
            {"kind": "Ordinary(Divw)", "seen": 1, "passed": 0, "failed": 1, "skipped": 0},
        ])
    );
    let unreached = json["unreached"].as_array().expect("an unreached list");
    assert!(unreached.contains(&serde_json::json!("Ordinary(Subf)")));
    assert!(!unreached.contains(&serde_json::json!("Ordinary(Divw)")));
    let failures = json["failures"].as_array().expect("a failure list");
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0]["record"], 0);
    assert_eq!(failures[0]["pc"], 0x1_0000);
    assert_eq!(failures[0]["raw"], DIVW);
    assert_eq!(failures[0]["kind"], "Ordinary(Divw)");
    assert_eq!(failures[0]["class"], "undefined");
    assert_eq!(failures[0]["detail"], "muldiv-undef");
    assert_eq!(failures[0]["fields"], serde_json::json!(["gpr3"]));
}
