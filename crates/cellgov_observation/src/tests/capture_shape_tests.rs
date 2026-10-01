//! The reference lookup refuses a file loose in `ps3/`, and the loader
//! refuses an observation whose outcome, events, TTY log or step count
//! a frame conversion never produces.

use super::tests::{
    capture_in, console_observation, profiles, provenance, test_dir, whole_capture, PROFILE,
};
use super::*;
use crate::observation::{ObservedEvent, ObservedEventKind};

#[test]
fn a_file_loose_in_the_capture_directory_is_an_error_not_an_absent_capture() {
    for loose in ["observation.json", PROFILE] {
        let test_dir = test_dir();
        std::fs::create_dir_all(test_dir.join(CAPTURE_DIR)).expect("ps3 dir");
        let path = test_dir.join(CAPTURE_DIR).join(loose);
        std::fs::write(&path, "{}").expect("loose file");
        match microtest_reference(&test_dir, &profiles()) {
            Err(HardwareCaptureError::LooseFile(found)) => assert_eq!(found, path),
            other => panic!("{loose}: {other:?}"),
        }
    }
}

#[test]
fn a_loose_file_beside_a_whole_capture_is_still_an_error() {
    let (test_dir, _dir) = whole_capture();
    let path = test_dir.join(CAPTURE_DIR).join(FRAME_FILE);
    std::fs::write(&path, b"CGOV").expect("loose file");
    assert!(matches!(
        microtest_reference(&test_dir, &profiles()),
        Err(HardwareCaptureError::LooseFile(found)) if found == path
    ));
    assert!(matches!(
        profile_directories(&test_dir),
        Err(HardwareCaptureError::LooseFile(_))
    ));
}

#[test]
fn the_profile_directories_are_listed_sorted() {
    let (test_dir, dir) = whole_capture();
    let other = test_dir.join(CAPTURE_DIR).join("cech20-cex-492");
    std::fs::create_dir_all(&other).expect("dir");
    assert_eq!(profile_directories(&test_dir).expect("lists"), [other, dir]);
}

#[test]
fn an_observation_a_frame_conversion_never_produces_is_refused() {
    type Edit = fn(&mut Observation);
    let edits: [(&str, Edit); 4] = [
        ("an outcome other than completed", |o| {
            o.outcome = ObservedOutcome::Timeout
        }),
        ("events", |o| {
            o.events.push(ObservedEvent {
                kind: ObservedEventKind::MailboxSend,
                unit: 0,
                sequence: 0,
            })
        }),
        ("a TTY log", |o| o.tty_log = b"x".to_vec()),
        ("a step count", |o| o.metadata.steps = Some(1)),
    ];
    for (field, edit) in edits {
        let mut observation = console_observation();
        edit(&mut observation);
        let (_test, dir) = capture_in(PROFILE, &provenance(), &observation);
        match load(&dir, &profiles()) {
            Err(HardwareCaptureError::ObservationField(found)) => assert_eq!(found, field),
            other => panic!("{field}: {other:?}"),
        }
    }
}
