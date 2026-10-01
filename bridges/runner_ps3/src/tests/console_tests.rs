//! The claimed profile, the status-page parse over a redacted copy of
//! the real page, the identity's hard and soft fields, and the check
//! against the tracked profiles file.

use std::path::Path;

use cellgov_observation::console_profile::{console_profiles_path, ConsoleProfileError};

use super::*;
use crate::load::parse_load;

/// webMAN's status page as the console served it, with every
/// identifier replaced by a synthetic value of the same shape.
const PAGE: &str = include_str!("fixtures/cpursx.html");

const PROFILE: &str = "cech20-cex-493";

fn tracked() -> ConsoleProfiles {
    let path = console_profiles_path(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    ConsoleProfiles::load(&path).expect("the tracked profiles load")
}

fn operator() -> OperatorFacts {
    OperatorFacts {
        model: Some("CECH-2001A".to_string()),
        cfw: Some("EvilNAT 4.93 PEX".to_string()),
        debugger_attached: Some(false),
    }
}

#[test]
fn the_flag_wins_over_the_variable() {
    assert_eq!(claimed_profile(Some("a"), Some("b")).expect("flag"), "a");
    assert_eq!(claimed_profile(None, Some("b")).expect("variable"), "b");
    assert_eq!(
        claimed_profile(Some(" "), Some(" b ")).expect("blank flag"),
        "b"
    );
}

#[test]
fn neither_source_is_a_usage_error_naming_both() {
    for (flag, env) in [(None, None), (Some(""), Some("")), (Some("  "), Some("\t"))] {
        let err = claimed_profile(flag, env).expect_err("unclaimed");
        assert_eq!(err.exit_code(), crate::ExitCode::Usage);
        assert_eq!(
            err.to_string(),
            "usage: no console profile claimed; pass --profile <name> or set CELLGOV_PS3_PROFILE"
        );
    }
}

#[test]
fn the_real_page_states_firmware_kernel_cobra_and_webman_and_no_identifier() {
    let page = parse_status_page(PAGE);
    assert_eq!(
        page,
        StatusPage {
            firmware: Some("4.93".to_string()),
            kernel: Some("cex".to_string()),
            cobra: Some("8.5".to_string()),
            webman: Some("1.47.48t".to_string()),
            model: None,
        }
    );
}

#[test]
fn the_real_page_and_the_operator_facts_satisfy_the_tracked_reference() {
    let mut transcript = Transcript::new();
    let facts = establish(PAGE, &operator(), &tracked(), PROFILE, &mut transcript)
        .expect("the console satisfies its profile");
    assert_eq!(
        facts,
        ConsoleFacts {
            profile: PROFILE.to_string(),
            model: "CECH-2001A".to_string(),
            kernel: "cex".to_string(),
            firmware: "4.93".to_string(),
            cfw: "EvilNAT 4.93 PEX".to_string(),
            cobra: "8.5".to_string(),
            webman: Some("1.47.48t".to_string()),
            debugger_attached: false,
            load_at_start: None,
        }
    );
    assert_eq!(
        transcript.lines(),
        [
            "#0001 = console: model CECH-2001A, kernel cex, firmware 4.93, cfw EvilNAT 4.93 PEX, \
             cobra 8.5, webman 1.47.48t, debugger not attached",
            "#0002 = console satisfies profile cech20-cex-493",
        ]
    );
}

#[test]
fn a_changed_soft_field_is_recorded_and_never_refuses() {
    let page = PAGE.replace("webMAN 1.47.48t", "webMAN 1.47.50");
    let mut transcript = Transcript::new();
    let facts = establish(&page, &operator(), &tracked(), PROFILE, &mut transcript)
        .expect("a webMAN update is soft");
    assert_eq!(facts.webman.as_deref(), Some("1.47.50"));
    assert!(transcript.lines()[0].contains("webman 1.47.50"));

    let page = PAGE.replace("webMAN 1.47.48t MOD", "Simple Web Server");
    let mut transcript = Transcript::new();
    let facts = establish(&page, &operator(), &tracked(), PROFILE, &mut transcript)
        .expect("an absent soft field is recorded as absent");
    assert_eq!(facts.webman, None);
    assert!(transcript.lines()[0].contains("webman not stated"));
}

#[test]
fn a_hard_field_the_page_does_not_state_is_a_transport_error_naming_it() {
    for (from, to, field) in [
        ("Firmware: 4.93 CEX Cobra 8.5", "Cobra 8.5", "firmware"),
        ("4.93 CEX Cobra", "4.93 XYZ Cobra", "kernel"),
        ("Cobra 8.5", "", "cobra"),
    ] {
        let page = PAGE.replace(from, to);
        match identify(&parse_status_page(&page), &operator(), PROFILE) {
            Err(err @ ConsoleError::PageMissing { field: named }) => {
                assert_eq!(named, field, "{from:?} -> {to:?}");
                assert_eq!(
                    RunnerPs3Error::from(err).exit_code(),
                    crate::ExitCode::Transport
                );
            }
            other => panic!("{field}: {other:?}"),
        }
    }
}

#[test]
fn a_hard_field_only_the_operator_states_is_a_usage_error_when_absent_or_blank() {
    for (blank, field) in [
        (
            OperatorFacts {
                debugger_attached: None,
                ..operator()
            },
            "debugger_attached",
        ),
        (
            OperatorFacts {
                model: Some("  ".to_string()),
                ..operator()
            },
            "model",
        ),
        (
            OperatorFacts {
                cfw: None,
                ..operator()
            },
            "cfw",
        ),
    ] {
        let err = identify(&parse_status_page(PAGE), &blank, PROFILE).expect_err(field);
        assert!(
            matches!(err, ConsoleError::OperatorMissing { field: named, .. } if named == field),
            "{err}"
        );
        assert_eq!(
            RunnerPs3Error::from(err).exit_code(),
            crate::ExitCode::Usage
        );
    }
}

#[test]
fn an_operator_model_fills_the_page_and_may_not_contradict_it() {
    let page = PAGE.replace("HDD:", "Model: CECH-2501A HDD:");
    match identify(&parse_status_page(&page), &operator(), PROFILE) {
        Err(ConsoleError::Contradiction {
            field,
            page,
            operator,
        }) => {
            assert_eq!((field, page.as_str()), ("model", "CECH-2501A"));
            assert_eq!(operator, "CECH-2001A");
        }
        other => panic!("{other:?}"),
    }
    let unstated = OperatorFacts {
        model: None,
        ..operator()
    };
    let facts =
        identify(&parse_status_page(&page), &unstated, PROFILE).expect("the page states it");
    assert_eq!(facts.model, "CECH-2501A");
}

#[test]
fn a_wrong_hard_field_fails_the_claimed_profile_naming_the_field() {
    let page = PAGE.replace("Firmware: 4.93", "Firmware: 4.92");
    let err = establish(
        &page,
        &operator(),
        &tracked(),
        PROFILE,
        &mut Transcript::new(),
    )
    .expect_err("4.92 is another profile");
    assert_eq!(err.exit_code(), crate::ExitCode::Refused);
    match err {
        RunnerPs3Error::Profile(ConsoleProfileError::Mismatch { mismatches, .. }) => {
            assert_eq!(mismatches.len(), 1);
            assert_eq!(mismatches[0].field, "firmware");
            assert_eq!(mismatches[0].observed, "4.92");
        }
        other => panic!("{other:?}"),
    }
}

fn facts_from_page(page: &str) -> ConsoleFacts {
    identify(&parse_status_page(page), &operator(), PROFILE).expect("identified")
}

/// The report for `page` under `claimed`, its load read from the page
/// and taken as cool.
fn report_of(
    page: &str,
    profiles: &ConsoleProfiles,
    claimed: &str,
) -> Result<(StatusReport, Result<(), ConsoleProfileError>), ConsoleProfileError> {
    status_report(
        &facts_from_page(page),
        profiles,
        claimed,
        parse_load(page),
        Some(Thermal::Cool),
    )
}

#[test]
fn the_status_report_gives_each_hard_field_a_verdict_and_the_soft_fields() {
    let (report, verdict) = report_of(PAGE, &tracked(), PROFILE).expect("tracked claim");
    verdict.expect("the console satisfies its claim");
    assert_eq!(report.verdict, StatusVerdict::Pass);
    assert_eq!(
        report.lines(),
        [
            "profile cech20-cex-493",
            "  models: CECH-2001A ok",
            "  kernel: cex ok",
            "  firmware: 4.93 ok",
            "  cfw: EvilNAT 4.93 PEX ok",
            "  cobra: 8.5 ok",
            "  debugger_attached: none ok",
            "soft: model CECH-2001A, cfw EvilNAT 4.93 PEX, webman 1.47.48t",
            "also satisfies: none",
            "load: Cell 67 C, RSX 64 C, fan not stated, /dev_hdd0 99840 MiB free",
            "thermal: cool",
        ]
    );
}

#[test]
fn the_status_report_marks_a_failing_field_and_names_a_profile_the_console_satisfies() {
    let two = ConsoleProfiles::parse(concat!(
        "reference = \"cech20-cex-493\"\n",
        "[load]\nhot_c = 80\ncool_c = 72\nhdd_floor_mib = 1024\nwait_poll_s = 15\nwait_limit_s = 1800\n",
        "[profile.cech20-cex-493]\nmodels = [\"CECH-20\"]\nkernel = \"cex\"\n",
        "firmware = \"4.93\"\ncfw = \"EvilNAT\"\ncobra = \"8.5\"\ndebugger_attached = false\n",
        "[profile.cech20-cex-493-cobra84]\nmodels = [\"CECH-20\"]\nkernel = \"cex\"\n",
        "firmware = \"4.93\"\ncfw = \"EvilNAT\"\ncobra = \"8.4\"\ndebugger_attached = false\n",
    ))
    .expect("profiles");
    let page = PAGE.replace("Cobra 8.5", "Cobra 8.4");
    let (report, verdict) = report_of(&page, &two, PROFILE).expect("tracked claim");
    assert!(matches!(verdict, Err(ConsoleProfileError::Mismatch { .. })));
    assert_eq!(report.also_satisfies, ["cech20-cex-493-cobra84"]);
    let lines = report.lines();
    assert_eq!(lines[5], "  cobra: 8.4 FAILS, the profile requires \"8.5\"");
    assert_eq!(lines[8], "also satisfies: cech20-cex-493-cobra84");
}

#[test]
fn the_status_report_of_an_untracked_claim_is_an_error() {
    assert!(matches!(
        report_of(PAGE, &tracked(), "cech25"),
        Err(ConsoleProfileError::UnknownProfile { .. })
    ));
}
