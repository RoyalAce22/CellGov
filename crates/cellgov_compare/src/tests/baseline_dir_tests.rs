//! Loading a directory of observations.

use super::*;
use crate::test_support::sample_observation;

fn observation(runner: &str) -> Observation {
    let mut obs = sample_observation();
    obs.metadata.runner = runner.to_string();
    obs
}

#[test]
fn only_json_files_load_and_they_load_in_name_order() {
    let dir = cellgov_testkit::scratch::scratch();
    save(&observation("second"), &dir.join("b.json")).expect("save b");
    save(&observation("first"), &dir.join("a.json")).expect("save a");
    std::fs::write(dir.join("notes.txt"), "not an observation").expect("write notes");
    let loaded = load_dir(&dir).expect("the directory loads");
    let names: Vec<String> = loaded
        .iter()
        .map(|(path, _)| {
            path.file_name()
                .expect("file name")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(names, ["a.json", "b.json"]);
    assert_eq!(loaded[0].1.metadata.runner, "first");
}

#[test]
fn a_json_file_that_is_no_observation_names_itself() {
    let dir = cellgov_testkit::scratch::scratch();
    std::fs::write(dir.join("bad.json"), "{ not json").expect("write bad");
    let err = load_dir(&dir).expect_err("a bad file refuses the directory");
    assert!(matches!(err, BaselineDirError::Load { .. }), "got {err:?}");
    assert!(err.to_string().contains("bad.json"), "got {err}");
}

#[test]
fn a_missing_directory_is_a_read_error() {
    let dir = cellgov_testkit::scratch::scratch();
    let err = load_dir(&dir.join("absent")).expect_err("no directory, no observations");
    assert!(
        matches!(err, BaselineDirError::ReadDir { .. }),
        "got {err:?}"
    );
}
