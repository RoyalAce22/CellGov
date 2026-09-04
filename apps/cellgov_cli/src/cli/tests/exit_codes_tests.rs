use super::*;

#[test]
fn the_contract_text_names_every_shared_code() {
    for (code, meaning) in [
        (FAILED, "the operation ran and failed"),
        (USAGE, "usage error"),
        (DISAGREED, "runs that had to reproduce each other disagreed"),
        (DIVERGED, "a subprocess failed, or a verification diverged"),
        (ANCHOR_MOVED, "a boot moved off its committed anchor"),
    ] {
        let line = format!("  {code}    {meaning}");
        assert!(
            CONTRACT.contains(&line),
            "the contract text has no line {line:?}:\n{CONTRACT}"
        );
    }
    assert!(
        CONTRACT.contains(&format!(">={FIRST_COMMAND_SPECIFIC}")),
        "the contract text does not name where command-specific codes start:\n{CONTRACT}"
    );
}

/// A const call fails the build, which no test can observe, so this
/// drives the same argument through a runtime call.
#[test]
#[should_panic(expected = "command-specific exit status")]
fn a_command_specific_status_may_not_take_a_shared_code() {
    let _ = command_specific(DIVERGED);
}

#[test]
fn the_first_command_specific_code_is_itself_accepted() {
    assert_eq!(
        command_specific(FIRST_COMMAND_SPECIFIC),
        FIRST_COMMAND_SPECIFIC
    );
}

#[test]
fn no_two_shared_codes_collide() {
    let mut codes = [FAILED, USAGE, DISAGREED, DIVERGED, ANCHOR_MOVED];
    codes.sort_unstable();
    assert!(
        codes.windows(2).all(|w| w[0] != w[1]),
        "two shared statuses share a code: {codes:?}"
    );
    assert!(
        codes.iter().all(|c| *c < FIRST_COMMAND_SPECIFIC),
        "a shared status sits in the command-specific range: {codes:?}"
    );
}
