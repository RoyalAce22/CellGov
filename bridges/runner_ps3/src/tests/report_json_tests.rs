//! Each report's JSON form: every variant round-trips, the console
//! verbs other than `capture` carry only their transcript, `convert`
//! prints its observation untouched, and the transcript's redaction
//! holds through the JSON path.

use cellgov_observation::hardware_capture::ConsoleFacts;

use super::*;
use crate::console::{FailedField, StatusVerdict};
use crate::transcript::{MASKED_ID, MASKED_MAC};

fn facts() -> ConsoleFacts {
    ConsoleFacts {
        profile: "cech20-cex-493".to_string(),
        model: "CECH-2001A".to_string(),
        kernel: "cex".to_string(),
        firmware: "4.93".to_string(),
        cfw: "EvilNAT 4.93 PEX".to_string(),
        cobra: "8.5".to_string(),
        webman: Some("1.47.48t".to_string()),
        debugger_attached: false,
    }
}

fn status(verdict: StatusVerdict) -> Report {
    Report::Status(StatusReport {
        claimed: "cech20-cex-493".to_string(),
        facts: facts(),
        verdict,
        also_satisfies: vec!["cech20-cex-493-cobra84".to_string()],
    })
}

fn transcript() -> Vec<String> {
    vec![
        "#0001 > GET /cpursx.ps3".to_string(),
        "#0002 = console satisfies profile cech20-cex-493".to_string(),
    ]
}

#[test]
fn every_report_round_trips_through_its_json() {
    for report in [
        Report::Unlock {
            host: "10.77.0.2".to_string(),
            removed: true,
        },
        status(StatusVerdict::Pass),
        status(StatusVerdict::Mismatch {
            failed: vec![FailedField {
                field: "firmware".to_string(),
                expected: "\"4.92\"".to_string(),
                observed: "4.93".to_string(),
            }],
        }),
        Report::Console {
            capture: None,
            transcript: transcript(),
        },
        Report::Console {
            capture: Some(CaptureReport {
                capture_id: "spu_fixed_value-0123456789ab".to_string(),
                out: PathBuf::from("tests/micro/spu_fixed_value/ps3/cech20-cex-493"),
                replaced: Some("the SPU image changed".to_string()),
            }),
            transcript: transcript(),
        },
    ] {
        let json = report.json().expect("serializes");
        let back: Report = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{e}: {json}"));
        assert_eq!(back, report, "{json}");
        assert_eq!(report.render(true).expect("renders"), [json]);
    }
}

#[test]
fn a_console_verb_without_a_capture_reports_only_its_transcript() {
    let report = Report::Console {
        capture: None,
        transcript: transcript(),
    };
    let value: serde_json::Value =
        serde_json::from_str(&report.json().expect("serializes")).expect("json");
    assert_eq!(value, serde_json::json!({ "transcript": transcript() }));
}

#[test]
fn the_status_json_names_the_claim_the_verdict_and_the_facts() {
    let value: serde_json::Value =
        serde_json::from_str(&status(StatusVerdict::Pass).json().expect("serializes"))
            .expect("json");
    assert_eq!(value["claimed"], "cech20-cex-493");
    assert_eq!(value["verdict"], serde_json::json!({ "result": "pass" }));
    assert_eq!(value["facts"]["firmware"], "4.93");
    assert_eq!(value["also_satisfies"][0], "cech20-cex-493-cobra84");
}

#[test]
fn convert_prints_its_observation_as_it_stands() {
    let report = Report::Convert {
        json: "{\n  \"outcome\": \"completed\"\n}".to_string(),
        out: None,
    };
    assert_eq!(
        report.json().expect("serializes"),
        "{\n  \"outcome\": \"completed\"\n}"
    );
}

#[test]
fn a_status_page_leaves_no_identifier_in_the_json_transcript() {
    let mut recorded = Transcript::new();
    recorded.reply(concat!(
        "<tr><td>IDPS</td><td>0000000100850009141C2F2E3D4A5B6C</td></tr>",
        "<tr><td>PSID:</td><td>7A6B5C4D3E2F1A0B9C8D7E6F5A4B3C2D</td></tr>",
        "<tr><td>MAC</td><td>00:1F:A7:12:34:56</td></tr>",
    ));
    let json = Report::Console {
        capture: None,
        transcript: recorded.lines().to_vec(),
    }
    .json()
    .expect("serializes");
    for identifier in [
        "0000000100850009141C2F2E3D4A5B6C",
        "7A6B5C4D3E2F1A0B9C8D7E6F5A4B3C2D",
        "00:1F:A7:12:34:56",
    ] {
        assert!(!json.contains(identifier), "{identifier} survived: {json}");
    }
    assert_eq!(json.matches(MASKED_ID).count(), 2, "{json}");
    assert_eq!(json.matches(MASKED_MAC).count(), 1, "{json}");
}
