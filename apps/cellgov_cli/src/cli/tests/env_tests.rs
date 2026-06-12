//! Boolean environment-variable parsing -- truthy/falsy vocabulary and garbage rejection.

use super::*;

#[test]
fn parse_env_bool_unset_is_false() {
    assert!(!parse_env_bool_inner("X", None).unwrap());
}

#[test]
fn parse_env_bool_empty_is_false() {
    assert!(!parse_env_bool_inner("X", Some(String::new())).unwrap());
}

#[test]
fn parse_env_bool_accepts_truthy_vocab() {
    for v in ["1", "true", "TRUE", "yes", "Yes", "on", "ON"] {
        assert!(
            parse_env_bool_inner("X", Some(v.to_string())).unwrap(),
            "{v:?} should be truthy"
        );
    }
}

#[test]
fn parse_env_bool_accepts_falsy_vocab() {
    for v in ["0", "false", "FALSE", "no", "off"] {
        assert!(
            !parse_env_bool_inner("X", Some(v.to_string())).unwrap(),
            "{v:?} should be falsy"
        );
    }
}

#[test]
fn parse_env_bool_rejects_garbage() {
    let err = parse_env_bool_inner("X", Some("maybe".to_string()))
        .unwrap_err()
        .to_string();
    assert!(err.contains("X="), "got: {err}");
    assert!(err.contains("maybe"), "got: {err}");
}
