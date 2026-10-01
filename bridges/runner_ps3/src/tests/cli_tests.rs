//! The command line: each verb and its flags, the refusals that name
//! what is wrong, and the defaults.

use super::*;

fn args(words: &[&str]) -> Vec<OsString> {
    words.iter().map(OsString::from).collect()
}

fn parsed(words: &[&str]) -> Command {
    parse(&args(words)).expect("parses")
}

fn refusal(words: &[&str]) -> String {
    match parse(&args(words)) {
        Err(RunnerPs3Error::Usage(message)) => message,
        other => panic!("{words:?}: {other:?}"),
    }
}

#[test]
fn a_capture_line_parses_into_every_flag() {
    let command = parsed(&[
        "capture",
        "--host",
        "10.77.0.2",
        "--profile",
        "cech20-cex-493",
        "--model",
        "CECH-2001A",
        "--cfw",
        "EvilNAT 4.93 PEX",
        "--debugger",
        "none",
        "--manifest",
        "tests/micro/spu_fixed_value/manifest.toml",
        "--harness-revision",
        "0123abcd",
        "--poll-ms",
        "250",
        "--reclaim",
        "--recapture",
        "--reason",
        "the SPU image changed",
    ]);
    assert_eq!(
        command,
        Command {
            verb: Some(Verb::Capture),
            host: Some("10.77.0.2".to_string()),
            profile: Some("cech20-cex-493".to_string()),
            model: Some("CECH-2001A".to_string()),
            cfw: Some("EvilNAT 4.93 PEX".to_string()),
            debugger: Some(false),
            manifest: Some(PathBuf::from("tests/micro/spu_fixed_value/manifest.toml")),
            harness_revision: Some("0123abcd".to_string()),
            poll_ms: Some(250),
            reclaim: true,
            recapture: true,
            reason: Some("the SPU image changed".to_string()),
            ..Command::default()
        }
    );
    assert_eq!(command.poll_ms(), 250);
    assert_eq!(
        command.operator(),
        OperatorFacts {
            model: Some("CECH-2001A".to_string()),
            cfw: Some("EvilNAT 4.93 PEX".to_string()),
            debugger_attached: Some(false),
        }
    );
}

#[test]
fn deploy_takes_the_reclaim_its_refusal_names() {
    assert!(parsed(&["deploy", "--manifest", "m", "--reclaim"]).reclaim);
}

#[test]
fn every_verb_has_its_word() {
    for verb in <Verb as strum::VariantArray>::VARIANTS {
        assert_eq!(parsed(&[verb.name()]).verb, Some(*verb));
    }
}

#[test]
fn the_defaults_fill_what_the_line_leaves_out() {
    let command = parsed(&["status"]);
    assert_eq!(command.profiles_path(), PathBuf::from(DEFAULT_PROFILES));
    assert_eq!(command.poll_ms(), DEFAULT_POLL_MS);
    assert_eq!(command.operator().debugger_attached, None);
    assert_eq!(command.host(Some(" 10.77.0.2 ")).expect("env"), "10.77.0.2");
    assert_eq!(
        parsed(&["status", "--host", "ps3"])
            .host(Some("10.77.0.2"))
            .expect("flag"),
        "ps3"
    );
    assert_eq!(
        command.host(Some("  ")).expect_err("blank").to_string(),
        "usage: no console named; pass --host or set CELLGOV_PS3_HOST"
    );
}

#[test]
fn a_malformed_line_is_a_usage_error_naming_the_problem() {
    for (words, said) in [
        (&[][..], "no verb given"),
        (&["play"][..], "unknown verb \"play\""),
        (
            &["status", "--frame", "x"][..],
            "status does not take --frame",
        ),
        (
            &["unlock", "--manifest", "m"][..],
            "unlock does not take --manifest",
        ),
        (&["status", "--host"][..], "--host needs a value"),
        (
            &["capture", "--out", "--reclaim"][..],
            "--out needs a value, not the flag --reclaim",
        ),
        (
            &["status", "--host", "--profile", "p"][..],
            "--host needs a value, not the flag --profile",
        ),
        (
            &["run", "--poll-ms", "0"][..],
            "--poll-ms must be greater than zero",
        ),
        (
            &["cleanup", "--reclaim"][..],
            "cleanup does not take --reclaim",
        ),
        (
            &["status", "--host", "a", "--host", "b"][..],
            "--host given more than once",
        ),
        (
            &["capture", "--reclaim", "--reclaim"][..],
            "--reclaim given more than once",
        ),
        (
            &["status", "10.77.0.2"][..],
            "unexpected argument \"10.77.0.2\"",
        ),
        (
            &["status", "--debugger", "maybe"][..],
            "--debugger takes none or attached",
        ),
        (
            &["run", "--poll-ms", "soon"][..],
            "--poll-ms \"soon\" is not a millisecond count",
        ),
        (
            &["capture", "--recapture"][..],
            "--recapture and --reason go together",
        ),
        (
            &["capture", "--reason", "x"][..],
            "--recapture and --reason go together",
        ),
    ] {
        let message = refusal(words);
        assert!(message.starts_with(said), "{words:?}: {message}");
    }
}

#[test]
fn a_missing_required_path_names_its_flag() {
    let command = parsed(&["convert"]);
    assert_eq!(
        command.frame().expect_err("absent").to_string(),
        "usage: --frame is required"
    );
    assert_eq!(
        command.manifest().expect_err("absent").to_string(),
        "usage: --manifest is required"
    );
}

#[cfg(windows)]
#[test]
fn an_argument_that_is_not_utf8_is_a_usage_error_naming_its_position() {
    use std::os::windows::ffi::OsStringExt;
    let bad = OsString::from_wide(&[0x0068, 0xD800]);
    let message = match parse(&[OsString::from("status"), bad]) {
        Err(RunnerPs3Error::Usage(message)) => message,
        other => panic!("{other:?}"),
    };
    assert!(message.starts_with("argument 2 is not UTF-8"), "{message}");
}

#[cfg(unix)]
#[test]
fn an_argument_that_is_not_utf8_is_a_usage_error_naming_its_position() {
    use std::os::unix::ffi::OsStringExt;
    let bad = OsString::from_vec(vec![b'h', 0xFF]);
    let message = match parse(&[OsString::from("status"), bad]) {
        Err(RunnerPs3Error::Usage(message)) => message,
        other => panic!("{other:?}"),
    };
    assert!(message.starts_with("argument 2 is not UTF-8"), "{message}");
}
