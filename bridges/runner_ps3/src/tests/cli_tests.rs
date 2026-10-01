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
    assert_eq!(
        command.profiles_path(Path::new("root")),
        Path::new("root")
            .join("tests/micro")
            .join("console_profiles.toml")
    );
    assert_eq!(
        parsed(&["status", "--profiles", "mine.toml"]).profiles_path(Path::new("root")),
        PathBuf::from("mine.toml"),
        "--profiles resolves as typed"
    );
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

/// The flags [`USAGE`] names for `verb`: its line and the indented
/// lines after it, with `(status flags)` standing for status's.
fn usage_flags(verb: Verb) -> std::collections::BTreeSet<String> {
    let mut lines = USAGE
        .lines()
        .skip_while(|line| !line.starts_with(&format!("  {} ", verb.name())));
    let first = lines
        .next()
        .unwrap_or_else(|| panic!("USAGE has no line for {}", verb.name()));
    let block: Vec<&str> = std::iter::once(first)
        .chain(lines.take_while(|line| line.starts_with("   ")))
        .collect();
    let mut flags = std::collections::BTreeSet::new();
    for word in block.join(" ").split_whitespace() {
        if word == "(status" {
            flags.extend(usage_flags(Verb::Status));
        }
        let word = word.trim_matches(|c| c == '[' || c == ']');
        if word.starts_with("--") {
            flags.insert(word.to_string());
        }
    }
    flags
}

#[test]
fn usage_names_exactly_the_flags_each_verb_takes() {
    for verb in <Verb as strum::VariantArray>::VARIANTS {
        let table: std::collections::BTreeSet<String> =
            verb.flags().iter().map(|flag| flag.to_string()).collect();
        assert_eq!(usage_flags(*verb), table, "{}", verb.name());
    }
}

#[test]
fn the_parser_refuses_every_flag_outside_the_verbs_table() {
    let every: std::collections::BTreeSet<&str> = <Verb as strum::VariantArray>::VARIANTS
        .iter()
        .flat_map(|verb| verb.flags())
        .copied()
        .collect();
    for verb in <Verb as strum::VariantArray>::VARIANTS {
        for flag in every.iter().filter(|flag| !verb.flags().contains(flag)) {
            assert_eq!(
                refusal(&[verb.name(), flag]),
                format!("{} does not take {flag}", verb.name())
            );
        }
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
