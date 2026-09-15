//! The strict boolean parse of a `CELLGOV_*` toggle.

use super::{parse_bool_value, EnvBoolError};

#[test]
fn unset_and_empty_read_false() {
    assert_eq!(parse_bool_value("X", None), Ok(false));
    assert_eq!(parse_bool_value("X", Some(String::new())), Ok(false));
    assert_eq!(parse_bool_value("X", Some("  ".to_string())), Ok(false));
}

#[test]
fn every_recognized_spelling_folds_case_and_space() {
    for s in ["1", "TRUE", " yes ", "On"] {
        assert_eq!(parse_bool_value("X", Some(s.to_string())), Ok(true), "{s}");
    }
    for s in ["0", "FALSE", " no ", "Off"] {
        assert_eq!(parse_bool_value("X", Some(s.to_string())), Ok(false), "{s}");
    }
}

#[test]
fn an_unrecognized_value_names_the_variable_and_what_it_held() {
    assert_eq!(
        parse_bool_value("CELLGOV_X", Some("Maybe".to_string())),
        Err(EnvBoolError {
            name: "CELLGOV_X".to_string(),
            got: "maybe".to_string(),
        })
    );
}
