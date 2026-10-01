//! Loading a committed console capture: the provenance record's own
//! checks, its profile directory and hard fields, the frame's length
//! and hash, the observation's shape, the transcript, and the reference
//! lookup's absent, present and malformed answers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cellgov_testkit::scratch::ScratchDir;

use super::*;
use crate::test_support::sample_observation;

const FRAME: &[u8] = b"CGOV\x00\x00\x00\x08\x00\x00\x00\x00\x00\x00\x00\x01";
/// SHA-256 of `FRAME`, computed outside the crate.
const FRAME_SHA256: &str = "a6a02b1911a2fd8c0fa63ac7afff4213a9c76e97501775919ae9a777cf917894";
const PROFILE: &str = "cech20-cex-493";
const TEST: &str = "spu_fixed_value";

const PROFILES: &str = r#"
reference = "cech20-cex-493"

[profile.cech20-cex-493]
models = ["CECH-20"]
kernel = "cex"
firmware = "4.93"
cfw = "EvilNAT"
cobra = "8.5"
debugger_attached = false

[profile.cech20-dex-493]
models = ["CECH-20"]
kernel = "dex"
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

fn provenance() -> CaptureProvenance {
    CaptureProvenance {
        schema: CAPTURE_PROVENANCE_SCHEMA,
        capture_id: CaptureProvenance::capture_id(TEST, FRAME_SHA256),
        captured_at: "2026-09-30T12:00:00Z".to_string(),
        console: ConsoleFacts {
            profile: PROFILE.to_string(),
            model: "CECH-2001A".to_string(),
            kernel: "cex".to_string(),
            firmware: "4.93".to_string(),
            cfw: "EvilNAT 4.93 PEX".to_string(),
            cobra: "8.5".to_string(),
            webman: Some("1.47.48".to_string()),
            debugger_attached: false,
        },
        transport: TransportFacts {
            kind: "webman-filedrop".to_string(),
        },
        harness: HarnessFacts {
            runner: "runner_ps3".to_string(),
            revision: "0123456789abcdef".to_string(),
            link: "https://example.invalid/0123456789abcdef".to_string(),
        },
        microtest: MicrotestFacts {
            name: TEST.to_string(),
            manifest_sha256: "a".repeat(64),
            sources: BTreeMap::from([("ppu/main.c".to_string(), "b".repeat(64))]),
        },
        artifacts: ArtifactHashes {
            eboot_sha256: "c".repeat(64),
            ps3_elf_sha256: "d".repeat(64),
            reference_elf_sha256: "e".repeat(64),
            siblings: BTreeMap::from([("spu_main.elf".to_string(), "f".repeat(64))]),
            param_sfo_sha256: "1".repeat(64),
        },
        frame: FrameFacts {
            file: FRAME_FILE.to_string(),
            sha256: FRAME_SHA256.to_string(),
            bytes: FRAME.len() as u64,
            result_path: "/dev_hdd0/tmp/cgov_spu_fixed_value.bin".to_string(),
        },
        digest: format!("{FRAME_SHA256}  micro:spu_fixed_value  {}", FRAME.len()),
        recapture_reason: None,
    }
}

fn console_observation() -> Observation {
    let mut observation = sample_observation();
    observation.metadata.runner = RUNNER_PS3_CEX.to_string();
    observation.state_hashes = None;
    observation.identity = Default::default();
    observation.runner_firmware = Some("4.93".to_string());
    observation
}

/// A test directory inside a scratch directory that lives as long as
/// this value does.
struct TestDir {
    _scratch: ScratchDir,
    path: PathBuf,
}

impl std::ops::Deref for TestDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

/// A scratch test directory named `name`.
fn test_dir_named(name: &str) -> TestDir {
    let scratch = cellgov_testkit::scratch::scratch();
    let path = scratch.join(name);
    std::fs::create_dir_all(&path).expect("test dir");
    TestDir {
        _scratch: scratch,
        path,
    }
}

/// A scratch test directory named for the test.
fn test_dir() -> TestDir {
    test_dir_named(TEST)
}

/// A whole capture under `profile` in a fresh test directory: the
/// directory's guard and the capture directory.
fn capture_in(
    profile: &str,
    provenance: &CaptureProvenance,
    observation: &Observation,
) -> (TestDir, PathBuf) {
    let test_dir = test_dir();
    let dir = write_capture(&test_dir, profile, provenance, observation);
    (test_dir, dir)
}

/// Write a whole capture under `<test_dir>/ps3/<profile>/` and return
/// that directory.
fn write_capture(
    test_dir: &Path,
    profile: &str,
    provenance: &CaptureProvenance,
    observation: &Observation,
) -> PathBuf {
    let dir = test_dir.join(CAPTURE_DIR).join(profile);
    std::fs::create_dir_all(&dir).expect("capture dir");
    std::fs::write(
        dir.join(PROVENANCE_FILE),
        serde_json::to_vec_pretty(provenance).expect("provenance json"),
    )
    .expect("write provenance");
    std::fs::write(dir.join(FRAME_FILE), FRAME).expect("write frame");
    std::fs::write(
        dir.join(OBSERVATION_FILE),
        serde_json::to_vec_pretty(observation).expect("observation json"),
    )
    .expect("write observation");
    std::fs::write(dir.join(TRANSCRIPT_FILE), "#0001 > GET /cpursx.ps3\n")
        .expect("write transcript");
    dir
}

fn whole_capture() -> (TestDir, PathBuf) {
    capture_in(PROFILE, &provenance(), &console_observation())
}

#[test]
fn the_hash_is_lowercase_hex_sha256() {
    assert_eq!(sha256_hex(FRAME), FRAME_SHA256);
}

#[test]
fn the_capture_id_carries_the_test_name_and_twelve_hex_digits_of_the_frame_hash() {
    assert_eq!(
        CaptureProvenance::capture_id(TEST, FRAME_SHA256),
        "micro:spu_fixed_value#a6a02b1911a2"
    );
}

#[test]
fn a_provenance_record_round_trips_through_json_and_refuses_an_unknown_field() {
    let record = provenance();
    let json = serde_json::to_string(&record).expect("serialize");
    let back: CaptureProvenance = serde_json::from_str(&json).expect("parse");
    assert_eq!(back, record);
    let with_extra = json.replacen("\"schema\":", "\"idps\":\"00\",\"schema\":", 1);
    let err = serde_json::from_str::<CaptureProvenance>(&with_extra).expect_err("refused");
    assert!(err.to_string().contains("unknown field `idps`"), "{err}");
}

#[test]
fn the_record_check_refuses_another_schema_a_foreign_id_another_frame_name_and_a_bad_hash() {
    provenance().check().expect("the sample is consistent");

    let mut other_schema = provenance();
    other_schema.schema = 2;
    assert!(matches!(
        other_schema.check(),
        Err(HardwareCaptureError::Schema {
            found: 2,
            expected: CAPTURE_PROVENANCE_SCHEMA
        })
    ));

    let mut foreign_id = provenance();
    foreign_id.capture_id = "micro:spu_fixed_value#ffffffffffff".to_string();
    match foreign_id.check() {
        Err(HardwareCaptureError::CaptureId { found, expected }) => {
            assert_eq!(found, "micro:spu_fixed_value#ffffffffffff");
            assert_eq!(expected, "micro:spu_fixed_value#a6a02b1911a2");
        }
        other => panic!("{other:?}"),
    }

    let mut other_frame = provenance();
    other_frame.frame.file = "frame.bin".to_string();
    assert!(matches!(
        other_frame.check(),
        Err(HardwareCaptureError::FrameFile { found }) if found == "frame.bin"
    ));

    for bad in [String::new(), FRAME_SHA256.to_uppercase(), "a".repeat(63)] {
        let mut record = provenance();
        record.frame.sha256 = bad.clone();
        record.capture_id = CaptureProvenance::capture_id(TEST, &bad);
        assert!(
            matches!(record.check(), Err(HardwareCaptureError::FrameHashShape(found)) if found == bad),
            "{bad:?}"
        );
    }
}

#[test]
fn a_whole_capture_under_the_reference_profile_is_the_reference() {
    let (test_dir, dir) = whole_capture();
    let reference = microtest_reference(&test_dir, &profiles()).expect("loads");
    let MicrotestReference::Hardware(capture) = reference else {
        panic!("the reference profile's capture is the reference");
    };
    assert_eq!(capture.dir, dir);
    assert_eq!(capture.frame, FRAME);
    assert_eq!(capture.provenance, provenance());
    assert_eq!(capture.observation.metadata.runner, RUNNER_PS3_CEX);
}

#[test]
fn no_capture_directory_means_the_emulators_stand_alone() {
    let test_dir = test_dir();
    assert_eq!(
        microtest_reference(&test_dir, &profiles()).expect("nothing there"),
        MicrotestReference::EmulatorOnly
    );
    std::fs::create_dir_all(test_dir.join(CAPTURE_DIR).join("cech20-cex-492")).expect("dir");
    assert_eq!(
        microtest_reference(&test_dir, &profiles()).expect("another profile only"),
        MicrotestReference::EmulatorOnly,
        "a capture under another profile is never the reference"
    );
}

#[test]
fn a_capture_path_that_is_not_a_directory_is_an_error() {
    let test_dir = test_dir();
    std::fs::write(test_dir.join(CAPTURE_DIR), "not a directory").expect("write");
    match microtest_reference(&test_dir, &profiles()) {
        Err(HardwareCaptureError::NotADirectory(path)) => {
            assert_eq!(path, test_dir.join(CAPTURE_DIR));
        }
        other => panic!("{other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn a_capture_link_whose_target_is_gone_is_an_error_not_an_absent_capture() {
    let test_dir = test_dir();
    std::os::unix::fs::symlink(test_dir.join("gone"), test_dir.join(CAPTURE_DIR)).expect("link");
    match microtest_reference(&test_dir, &profiles()) {
        Err(HardwareCaptureError::Io { path, .. }) => {
            assert_eq!(path, test_dir.join(CAPTURE_DIR));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_capture_missing_a_file_is_an_error_naming_it() {
    for file in [OBSERVATION_FILE, TRANSCRIPT_FILE, FRAME_FILE] {
        let (test_dir, dir) = whole_capture();
        std::fs::remove_file(dir.join(file)).expect("remove");
        match microtest_reference(&test_dir, &profiles()) {
            Err(HardwareCaptureError::Io { path, .. }) => assert_eq!(path, dir.join(file)),
            other => panic!("{file}: {other:?}"),
        }
    }
}

#[test]
fn a_frame_of_another_length_than_the_record_states_is_refused_either_way() {
    let actual = FRAME.len() as u64;
    for recorded in [actual + 1, actual - 1] {
        let mut record = provenance();
        record.frame.bytes = recorded;
        let (_test, dir) = capture_in(PROFILE, &record, &console_observation());
        let err = load(&dir, &profiles()).expect_err("refused");
        assert!(
            matches!(
                err,
                HardwareCaptureError::FrameLength { found, expected }
                    if found == actual && expected == recorded
            ),
            "recorded {recorded}: {err:?}"
        );
    }
}

#[test]
fn a_frame_of_the_same_length_but_other_bytes_is_refused_by_its_hash() {
    let (_test, dir) = whole_capture();
    let mut changed = FRAME.to_vec();
    *changed.last_mut().expect("bytes") ^= 1;
    std::fs::write(dir.join(FRAME_FILE), &changed).expect("rewrite");
    match load(&dir, &profiles()) {
        Err(HardwareCaptureError::FrameHash { recorded, .. }) => {
            assert_eq!(recorded, FRAME_SHA256);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_capture_in_another_profiles_directory_is_refused() {
    let (_test, dir) = capture_in("cech20-cex-492", &provenance(), &console_observation());
    match load(&dir, &profiles()) {
        Err(HardwareCaptureError::ProfileDirectory { directory, profile }) => {
            assert_eq!(
                (directory.as_str(), profile.as_str()),
                ("cech20-cex-492", PROFILE)
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_capture_whose_console_fails_its_profile_or_names_an_untracked_one_is_refused() {
    let mut wrong_firmware = provenance();
    wrong_firmware.console.firmware = "4.92".to_string();
    let mut observation = console_observation();
    observation.runner_firmware = Some("4.92".to_string());
    let (_test, dir) = capture_in(PROFILE, &wrong_firmware, &observation);
    assert!(matches!(
        load(&dir, &profiles()),
        Err(HardwareCaptureError::Profile(
            ConsoleProfileError::Mismatch { .. }
        ))
    ));

    let mut untracked = provenance();
    untracked.console.profile = "cech25-dex-493".to_string();
    let (_test, dir) = capture_in("cech25-dex-493", &untracked, &console_observation());
    assert!(matches!(
        load(&dir, &profiles()),
        Err(HardwareCaptureError::Profile(
            ConsoleProfileError::UnknownProfile { .. }
        ))
    ));
}

#[test]
fn a_dex_console_under_the_cex_runner_string_is_refused() {
    let mut dex = provenance();
    dex.console.profile = "cech20-dex-493".to_string();
    dex.console.kernel = "dex".to_string();
    let (_test, dir) = capture_in("cech20-dex-493", &dex, &console_observation());
    assert!(matches!(
        load(&dir, &profiles()),
        Err(HardwareCaptureError::Kernel { found }) if found == "dex"
    ));
}

#[test]
fn a_soft_field_difference_still_loads() {
    let mut record = provenance();
    record.console.webman = None;
    record.console.model = "CECH-2004B".to_string();
    let (_test, dir) = capture_in(PROFILE, &record, &console_observation());
    load(&dir, &profiles()).expect("soft fields never refuse");
}

#[test]
fn a_capture_under_another_tests_directory_is_refused() {
    let other_test = test_dir_named("dma_completion");
    write_capture(&other_test, PROFILE, &provenance(), &console_observation());
    match microtest_reference(&other_test, &profiles()) {
        Err(HardwareCaptureError::TestName { found, expected }) => {
            assert_eq!(
                (found.as_str(), expected.as_str()),
                (TEST, "dma_completion")
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_observation_that_does_not_match_a_console_capture_is_refused() {
    let other_runner = sample_observation();
    assert_ne!(other_runner.metadata.runner, RUNNER_PS3_CEX);
    let (_test, dir) = capture_in(PROFILE, &provenance(), &other_runner);
    assert!(matches!(
        load(&dir, &profiles()),
        Err(HardwareCaptureError::Runner { .. })
    ));

    let mut other_firmware = console_observation();
    other_firmware.runner_firmware = None;
    let (_test, dir) = capture_in(PROFILE, &provenance(), &other_firmware);
    assert!(matches!(
        load(&dir, &profiles()),
        Err(HardwareCaptureError::ObservationFirmware {
            observation: None,
            ..
        })
    ));

    let mut with_hashes = console_observation();
    with_hashes.state_hashes = sample_observation().state_hashes;
    assert!(with_hashes.state_hashes.is_some());
    let (_test, dir) = capture_in(PROFILE, &provenance(), &with_hashes);
    assert!(matches!(
        load(&dir, &profiles()),
        Err(HardwareCaptureError::ObservationField("state hashes"))
    ));
}

#[test]
fn a_provenance_file_that_is_not_json_names_itself() {
    let (_test, dir) = whole_capture();
    std::fs::write(dir.join(PROVENANCE_FILE), "{ nope").expect("write");
    match load(&dir, &profiles()) {
        Err(HardwareCaptureError::Json { path, .. }) => {
            assert_eq!(path, dir.join(PROVENANCE_FILE));
        }
        other => panic!("{other:?}"),
    }
}
