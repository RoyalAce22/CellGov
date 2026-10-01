//! The claimed profile: the flag over the variable, an empty value as
//! absent, and a usage error naming both when neither is set.

use super::*;

#[test]
fn the_flag_wins_over_the_variable() {
    assert_eq!(claimed_profile(Some("a"), Some("b")).expect("flag"), "a");
    assert_eq!(claimed_profile(None, Some("b")).expect("variable"), "b");
    assert_eq!(
        claimed_profile(Some(""), Some("b")).expect("empty flag"),
        "b"
    );
}

#[test]
fn neither_source_is_a_usage_error_naming_both() {
    for (flag, env) in [(None, None), (Some(""), Some(""))] {
        let err = claimed_profile(flag, env).expect_err("unclaimed");
        assert_eq!(err.exit_code(), crate::ExitCode::Usage);
        assert_eq!(
            err.to_string(),
            "usage: no console profile claimed; pass --profile <name> or set CELLGOV_PS3_PROFILE"
        );
    }
}
