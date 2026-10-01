//! The frame check, the region mapping, the conversion, the capture
//! loop against an in-memory console, and the replay of a committed
//! capture, including the two changes it must refuse: one byte of the
//! frame, and a capture moved under another profile.

use cellgov_observation::console_profile::ConsoleProfiles;
use cellgov_observation::hardware_capture::{HardwareCaptureError, TRANSCRIPT_FILE};
use cellgov_testkit::scratch::ScratchDir;

use super::*;
use crate::memory_console::{package_on_disk, MemoryConsole, PACKAGED_FRAME};

const PROFILE: &str = "cech20-cex-493";

const PROFILES: &str = r#"
reference = "cech20-cex-493"

[profile.cech20-cex-493]
models = ["CECH-20"]
kernel = "cex"
firmware = "4.93"
cfw = "EvilNAT"
cobra = "8.5"
debugger_attached = false

[profile.cech20-cex-492]
models = ["CECH-20"]
kernel = "cex"
firmware = "4.92"
cfw = "EvilNAT"
cobra = "8.5"
debugger_attached = false
"#;

fn profiles() -> ConsoleProfiles {
    ConsoleProfiles::parse(PROFILES).expect("profiles")
}

fn facts() -> ConsoleFacts {
    ConsoleFacts {
        profile: PROFILE.to_string(),
        model: "CECH-2001A".to_string(),
        kernel: "cex".to_string(),
        firmware: "4.93".to_string(),
        cfw: "EvilNAT 4.93 PEX".to_string(),
        cobra: "8.5".to_string(),
        webman: Some("1.47.48t".to_string()),
        debugger_attached: false,
    }
}

fn plan() -> (ScratchDir, CapturePlan) {
    let scratch = cellgov_testkit::scratch::scratch();
    let manifest_path = package_on_disk(&scratch);
    let manifest = manifest::load_console(&manifest_path).expect("manifest");
    let out = manifest_path
        .parent()
        .expect("test dir")
        .join("ps3")
        .join(PROFILE);
    let plan = CapturePlan {
        manifest_path,
        manifest,
        out,
        harness_revision: "0123456789abcdef".to_string(),
        poll_ms: 500,
        reclaim: false,
        keep_deployed: false,
        recapture_reason: None,
        clear_with: "runner_ps3 cleanup --host 10.77.0.2".to_string(),
    };
    (scratch, plan)
}

/// A clean console whose started test writes `frame`.
fn console_writing(plan: &CapturePlan, frame: &[u8]) -> MemoryConsole {
    let mut console = MemoryConsole::empty();
    console.on_start = Some((plan.target().result_path, frame.to_vec()));
    console.polls_before_result = 1;
    console
}

fn run_capture(
    console: &mut MemoryConsole,
    plan: &CapturePlan,
) -> Result<CaptureProvenance, RunnerPs3Error> {
    capture(
        console,
        plan,
        &facts(),
        &mut |_| {},
        &mut || Ok("2026-09-30T12:00:00Z".to_string()),
        &mut |_| Ok(true),
        &mut Transcript::new(),
    )
}

#[test]
fn only_one_whole_frame_passes_the_frame_check() {
    check_frame(PACKAGED_FRAME).expect("whole");
    for (bytes, said) in [
        (&b"CGOV\x00\x00"[..], "shorter than the 8-byte header"),
        (&b"VOGC\x00\x00\x00\x00"[..], "CGOV magic"),
        (
            &b"CGOV\x00\x00\x00\x04\x01\x02\x03"[..],
            "4-byte payload but 3",
        ),
        (
            &b"CGOV\x00\x00\x00\x02\x01\x02\x03"[..],
            "2-byte payload but 3",
        ),
    ] {
        match check_frame(bytes) {
            Err(RunnerPs3Error::Frame(message)) => assert!(message.contains(said), "{message}"),
            other => panic!("{said}: {other:?}"),
        }
    }
}

#[test]
fn a_manifest_that_cannot_make_a_comparable_observation_is_refused() {
    let base = plan().1.manifest;
    type Edit = fn(&mut ConsoleManifest);
    let edits: [(&str, Edit); 4] = [
        ("declares no region", |m| m.observe.memory_regions.clear()),
        ("names space 1", |m| m.observe.memory_regions[0].space = 1),
        ("declares zero bytes", |m| {
            m.observe.memory_regions[0].size = 0
        }),
        ("is declared twice", |m| {
            let copy = m.observe.memory_regions[0].clone();
            m.observe.memory_regions.push(copy);
        }),
    ];
    for (said, edit) in edits {
        let mut manifest = base.clone();
        edit(&mut manifest);
        match payload_regions(&manifest) {
            Err(RunnerPs3Error::Usage(message)) => assert!(message.contains(said), "{message}"),
            other => panic!("{said}: {other:?}"),
        }
    }
}

#[test]
fn a_frame_converts_to_the_consoles_observation_of_the_declared_regions() {
    let dir = cellgov_testkit::scratch::scratch();
    let frame = dir.join(FRAME_FILE);
    std::fs::write(&frame, PACKAGED_FRAME).expect("frame");
    let observation = frame_to_observation(&frame, &plan().1.manifest, "4.93").expect("converts");
    assert_eq!(observation.outcome, ObservedOutcome::Completed);
    assert_eq!(observation.memory_regions.len(), 1);
    assert_eq!(observation.memory_regions[0].name, "result");
    assert_eq!(observation.memory_regions[0].addr, 0);
    assert_eq!(
        observation.memory_regions[0].data,
        b"\x00\x00\x00\x00\x13\x37\xBA\xAD"
    );
    assert_eq!(observation.metadata.runner, RUNNER_PS3_CEX);
    assert_eq!(observation.runner_firmware.as_deref(), Some("4.93"));
    assert!(observation.state_hashes.is_none());
    assert!(observation.identity.is_empty());
}

#[test]
fn a_capture_writes_four_files_that_replay_and_leaves_the_console_restored() {
    let (_scratch, plan) = plan();
    let mut console = console_writing(&plan, PACKAGED_FRAME);
    let record = run_capture(&mut console, &plan).expect("captured");
    assert_eq!(record.console.profile, PROFILE);
    assert_eq!(
        record.capture_id,
        record
            .frame
            .sha256
            .get(..12)
            .map_or_else(String::new, |digits| format!(
                "micro:spu_fixed_value#{digits}"
            ))
    );
    assert_eq!(
        record.microtest.sources.keys().collect::<Vec<_>>(),
        ["ppu/main.c"],
        "the build and capture trees are not sources"
    );
    for file in [
        FRAME_FILE,
        OBSERVATION_FILE,
        PROVENANCE_FILE,
        TRANSCRIPT_FILE,
    ] {
        assert!(plan.out.join(file).is_file(), "{file}");
    }
    assert_eq!(
        std::fs::read(plan.out.join(FRAME_FILE)).expect("frame"),
        PACKAGED_FRAME
    );
    replay(&plan.out, &plan.manifest_path, &profiles()).expect("the capture replays");
    assert_eq!(console.dirs.iter().collect::<Vec<_>>(), ["/dev_hdd0/game"]);
    assert!(console.files.is_empty(), "{:?}", console.files);
    let transcript = std::fs::read_to_string(plan.out.join(TRANSCRIPT_FILE)).expect("transcript");
    assert!(transcript.contains("= console restored"), "{transcript}");
}

#[test]
fn an_existing_capture_is_refused_unless_recaptured_with_a_reason() {
    let (_scratch, mut plan) = plan();
    std::fs::create_dir_all(&plan.out).expect("out");
    let mut console = console_writing(&plan, PACKAGED_FRAME);
    match run_capture(&mut console, &plan) {
        Err(RunnerPs3Error::Refused { clear_with, .. }) => {
            assert!(clear_with.contains("--recapture --reason"), "{clear_with}");
        }
        other => panic!("{other:?}"),
    }
    assert!(console.calls.is_empty(), "a refusal sends nothing");

    plan.recapture_reason = Some("the SPU image changed".to_string());
    let record = run_capture(&mut console, &plan).expect("recaptured");
    assert_eq!(
        record.recapture_reason.as_deref(),
        Some("the SPU image changed")
    );
}

#[test]
fn a_run_that_never_writes_its_result_times_out_cleans_up_and_writes_nothing() {
    let (_scratch, plan) = plan();
    let mut console = MemoryConsole::empty();
    let err = run_capture(&mut console, &plan).expect_err("no result");
    assert_eq!(err.exit_code(), crate::ExitCode::Timeout);
    assert!(!plan.out.exists());
    assert_eq!(console.dirs.iter().collect::<Vec<_>>(), ["/dev_hdd0/game"]);
    assert!(console.files.is_empty());
}

#[test]
fn a_result_that_is_not_one_frame_is_a_frame_error_and_the_console_is_cleaned() {
    let (_scratch, plan) = plan();
    let mut console = console_writing(&plan, b"CGOV\x00\x00\x00\x08short");
    let err = run_capture(&mut console, &plan).expect_err("short frame");
    assert_eq!(err.exit_code(), crate::ExitCode::Frame);
    assert!(!plan.out.exists());
    assert!(console.files.is_empty(), "{:?}", console.files);
}

#[test]
fn a_recapture_whose_frame_does_not_convert_leaves_the_old_capture_whole() {
    let (_scratch, mut plan) = plan();
    run_capture(&mut console_writing(&plan, PACKAGED_FRAME), &plan).expect("first capture");
    plan.recapture_reason = Some("retake".to_string());
    let short = b"CGOV\x00\x00\x00\x04\x00\x00\x00\x00";
    let mut console = console_writing(&plan, short);
    let err = run_capture(&mut console, &plan).expect_err("4 bytes cannot hold an 8-byte region");
    assert_eq!(err.exit_code(), crate::ExitCode::Frame);
    replay(&plan.out, &plan.manifest_path, &profiles()).expect("the old capture still replays");
    assert!(
        !staging_dir(&plan.out).exists(),
        "no staging directory remains"
    );
    assert!(console.files.is_empty(), "{:?}", console.files);
}

#[test]
fn a_cleanup_failure_after_the_capture_keeps_the_capture_and_exits_six() {
    let (_scratch, plan) = plan();
    let mut console = console_writing(&plan, PACKAGED_FRAME);
    console.unmount_status = Some(500);
    let err = run_capture(&mut console, &plan).expect_err("unmount fails");
    assert_eq!(err.exit_code(), crate::ExitCode::Cleanup);
    replay(&plan.out, &plan.manifest_path, &profiles()).expect("the capture still stands");
}

#[test]
fn replay_refuses_a_changed_frame_byte() {
    let (_scratch, plan) = plan();
    run_capture(&mut console_writing(&plan, PACKAGED_FRAME), &plan).expect("captured");
    let mut frame = PACKAGED_FRAME.to_vec();
    *frame.last_mut().expect("bytes") ^= 1;
    std::fs::write(plan.out.join(FRAME_FILE), frame).expect("rewrite");
    assert!(matches!(
        replay(&plan.out, &plan.manifest_path, &profiles()),
        Err(RunnerPs3Error::Capture(
            HardwareCaptureError::FrameHash { .. }
        ))
    ));
}

#[test]
fn replay_refuses_a_capture_moved_under_another_profile() {
    let (_scratch, plan) = plan();
    run_capture(&mut console_writing(&plan, PACKAGED_FRAME), &plan).expect("captured");
    let moved = plan.out.with_file_name("cech20-cex-492");
    std::fs::rename(&plan.out, &moved).expect("rename");
    assert!(matches!(
        replay(&moved, &plan.manifest_path, &profiles()),
        Err(RunnerPs3Error::Capture(
            HardwareCaptureError::ProfileDirectory { .. }
        ))
    ));
}

#[test]
fn replay_refuses_an_observation_its_frame_does_not_reproduce() {
    let (_scratch, plan) = plan();
    run_capture(&mut console_writing(&plan, PACKAGED_FRAME), &plan).expect("captured");
    let path = plan.out.join(OBSERVATION_FILE);
    let text = std::fs::read_to_string(&path).expect("observation");
    std::fs::write(&path, format!("{text}\n")).expect("rewrite");
    match replay(&plan.out, &plan.manifest_path, &profiles()) {
        Err(RunnerPs3Error::Frame(message)) => {
            assert!(
                message.ends_with("does not reproduce from its frame"),
                "{message}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn convert_names_an_untracked_profile() {
    let (_scratch, plan) = plan();
    let dir = cellgov_testkit::scratch::scratch();
    let frame = dir.join(FRAME_FILE);
    std::fs::write(&frame, PACKAGED_FRAME).expect("frame");
    let observation =
        convert(&frame, &plan.manifest_path, &profiles(), "cech20-cex-492").expect("tracked");
    assert_eq!(observation.runner_firmware.as_deref(), Some("4.92"));
    assert!(matches!(
        convert(&frame, &plan.manifest_path, &profiles(), "cech25"),
        Err(RunnerPs3Error::Profile(
            ConsoleProfileError::UnknownProfile { .. }
        ))
    ));
}
