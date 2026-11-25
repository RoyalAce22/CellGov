use super::tests::{profiles, whole_capture};
use super::*;

#[test]
fn a_loaded_capture_carries_its_transcript_bytes() {
    let (_test_dir, dir) = whole_capture();
    let capture = load(&dir, &profiles()).expect("the capture loads");
    assert_eq!(capture.transcript, b"#0001 > GET /cpursx.ps3\n");
}
