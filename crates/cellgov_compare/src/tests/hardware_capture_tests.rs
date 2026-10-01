//! Loading a committed console capture: the provenance record's own
//! checks, the frame length, the runner string, and the absent-directory
//! answer.

use std::collections::BTreeMap;
use std::path::Path;

use super::*;
use crate::test_support::sample_observation;

const FRAME: &[u8] = b"CGOV\x00\x00\x00\x08\x00\x00\x00\x00\x00\x00\x00\x01";
const FRAME_SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn provenance() -> CaptureProvenance {
    CaptureProvenance {
        schema: CAPTURE_PROVENANCE_SCHEMA,
        capture_id: CaptureProvenance::capture_id("spu_fixed_value", FRAME_SHA256),
        captured_at: "2026-09-30T12:00:00Z".to_string(),
        console: ConsoleFacts {
            profile: "cech20-cex-493".to_string(),
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
            name: "spu_fixed_value".to_string(),
            manifest_sha256: "a".repeat(64),
            sources: BTreeMap::from([("ppu/main.c".to_string(), "b".repeat(64))]),
        },
        artifacts: ArtifactHashes {
            eboot_sha256: "c".repeat(64),
            ps3_elf_sha256: "d".repeat(64),
            reference_elf_sha256: "e".repeat(64),
            spu_elf_sha256: Some("f".repeat(64)),
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

/// Write a whole capture under `<test_dir>/ps3/` and return that directory.
fn write_capture(test_dir: &Path, provenance: &CaptureProvenance, observation: &Observation) {
    let dir = test_dir.join(CAPTURE_DIR);
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
}

#[test]
fn the_capture_id_carries_the_test_name_and_twelve_hex_digits_of_the_frame_hash() {
    assert_eq!(
        CaptureProvenance::capture_id("spu_fixed_value", FRAME_SHA256),
        "micro:spu_fixed_value#0123456789ab"
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
    assert!(err.to_string().contains("idps"), "{err}");
}

#[test]
fn the_record_check_refuses_another_schema_a_foreign_id_and_another_frame_name() {
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
            assert_eq!(expected, "micro:spu_fixed_value#0123456789ab");
        }
        other => panic!("{other:?}"),
    }

    let mut other_frame = provenance();
    other_frame.frame.file = "frame.bin".to_string();
    assert!(matches!(
        other_frame.check(),
        Err(HardwareCaptureError::FrameFile { found }) if found == "frame.bin"
    ));
}

#[test]
fn a_whole_capture_loads_and_is_the_reference() {
    let test_dir = cellgov_testkit::scratch::scratch();
    write_capture(&test_dir, &provenance(), &console_observation());
    let reference = microtest_reference(&test_dir).expect("loads");
    let MicrotestReference::Hardware(capture) = reference else {
        panic!("a capture directory is the reference");
    };
    assert_eq!(capture.dir, test_dir.join(CAPTURE_DIR));
    assert_eq!(capture.frame, FRAME);
    assert_eq!(capture.provenance, provenance());
    assert_eq!(capture.observation.metadata.runner, RUNNER_PS3_CEX);
    assert_eq!(capture.observation.runner_firmware.as_deref(), Some("4.93"));
    assert!(capture.observation.identity.is_empty());
}

#[test]
fn no_capture_directory_means_the_emulators_stand_alone() {
    let test_dir = cellgov_testkit::scratch::scratch();
    assert_eq!(
        microtest_reference(&test_dir).expect("no directory is an answer"),
        MicrotestReference::EmulatorOnly
    );
}

#[test]
fn a_capture_directory_missing_a_file_is_an_error_not_emulator_only() {
    let test_dir = cellgov_testkit::scratch::scratch();
    write_capture(&test_dir, &provenance(), &console_observation());
    std::fs::remove_file(test_dir.join(CAPTURE_DIR).join(OBSERVATION_FILE)).expect("remove");
    let err = microtest_reference(&test_dir).expect_err("refused");
    match err {
        HardwareCaptureError::Io { path, .. } => {
            assert_eq!(path, test_dir.join(CAPTURE_DIR).join(OBSERVATION_FILE));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_frame_of_another_length_than_the_record_states_is_refused_either_way() {
    let actual = FRAME.len() as u64;
    for recorded in [actual + 1, actual - 1] {
        let test_dir = cellgov_testkit::scratch::scratch();
        let mut record = provenance();
        record.frame.bytes = recorded;
        write_capture(&test_dir, &record, &console_observation());
        let err = load(&test_dir.join(CAPTURE_DIR)).expect_err("refused");
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
fn an_observation_of_another_runner_is_refused() {
    let test_dir = cellgov_testkit::scratch::scratch();
    let observation = sample_observation();
    let runner = observation.metadata.runner.clone();
    assert_ne!(runner, RUNNER_PS3_CEX);
    write_capture(&test_dir, &provenance(), &observation);
    let err = load(&test_dir.join(CAPTURE_DIR)).expect_err("refused");
    assert!(
        matches!(&err, HardwareCaptureError::Runner { found } if found == &runner),
        "{err:?}"
    );
}

#[test]
fn a_provenance_file_that_is_not_json_names_itself() {
    let test_dir = cellgov_testkit::scratch::scratch();
    write_capture(&test_dir, &provenance(), &console_observation());
    std::fs::write(test_dir.join(CAPTURE_DIR).join(PROVENANCE_FILE), "{ nope").expect("write");
    let err = load(&test_dir.join(CAPTURE_DIR)).expect_err("refused");
    match err {
        HardwareCaptureError::Json { path, .. } => {
            assert_eq!(path, test_dir.join(CAPTURE_DIR).join(PROVENANCE_FILE));
        }
        other => panic!("{other:?}"),
    }
}
