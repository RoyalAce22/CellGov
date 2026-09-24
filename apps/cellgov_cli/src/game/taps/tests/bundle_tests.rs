//! Two watches naming one capture are refused before either file
//! exists.

use super::open;
use crate::game::taps::error::TapError;
use crate::game::taps::specs::{parse_sample, parse_store};

#[test]
fn two_watches_naming_one_capture_are_refused_before_either_file_exists() {
    let dir = cellgov_testkit::scratch::scratch_labeled("taps_bundle_shared");
    let path = dir.join("shared.bin");
    let same = dir.join(".").join("shared.bin");
    let store = parse_store(Some("1000:10"), path.to_str())
        .unwrap()
        .unwrap();
    let sample = parse_sample(Some("100:4"), same.to_str(), None)
        .unwrap()
        .unwrap();
    let err = open(None, Some(store), Some(sample))
        .err()
        .expect("a shared capture path is refused");
    assert!(
        matches!(
            err,
            TapError::SharedCapture {
                first: "store-watch",
                second: "value-sample",
                ..
            }
        ),
        "{err}"
    );
    assert!(!path.exists(), "no capture is created");
}
