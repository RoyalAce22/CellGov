//! Identity-triple round-trips, fingerprints, and the cross-triple
//! report.

use super::*;
use crate::test_support::identity;

fn fw_only(version: &str) -> RunIdentity {
    RunIdentity {
        firmware: Some(FirmwareIdentity {
            version: version.into(),
            image_version: "0x0004009300000000".into(),
            pup_sha256: "ab".repeat(32),
        }),
        game: None,
        overrides: Default::default(),
    }
}

#[test]
fn json_round_trip_preserves_both_halves() {
    let id = identity("4.91", "NPAA00001", "update:02.51");
    let text = serde_json::to_string(&id).expect("serialize");
    let back: RunIdentity = serde_json::from_str(&text).expect("deserialize");
    assert_eq!(back, id);
}

#[test]
fn an_empty_identity_serializes_to_an_empty_object() {
    let text = serde_json::to_string(&RunIdentity::default()).expect("serialize");
    assert_eq!(text, "{}");
}

#[test]
fn an_absent_half_reads_back_as_none() {
    let text = serde_json::to_string(&fw_only("4.93")).expect("serialize");
    let back: RunIdentity = serde_json::from_str(&text).expect("deserialize");
    assert!(back.game.is_none());
    assert_eq!(back.firmware.map(|f| f.version).as_deref(), Some("4.93"));
}

#[test]
fn an_absent_half_fingerprints_as_zero() {
    let id = fw_only("4.93");
    assert_eq!(id.game_fingerprint(), 0);
    assert_ne!(id.firmware_fingerprint(), 0);
}

#[test]
fn two_firmware_versions_fingerprint_apart() {
    assert_ne!(
        fw_only("4.91").firmware_fingerprint(),
        fw_only("4.93").firmware_fingerprint()
    );
}

#[test]
fn field_boundaries_are_hashed() {
    let split = |title_id: &str, version: &str| {
        RunIdentity {
            firmware: None,
            game: Some(GameIdentity {
                title_id: title_id.into(),
                version: version.into(),
                app_version: None,
            }),
            overrides: Default::default(),
        }
        .game_fingerprint()
    };
    assert_ne!(split("NPAA0", "0001"), split("NPAA", "00001"));
}

#[test]
fn a_field_that_embeds_the_separator_still_fingerprints_apart() {
    let split = |title_id: &str, version: &str| {
        RunIdentity {
            firmware: None,
            game: Some(GameIdentity {
                title_id: title_id.into(),
                version: version.into(),
                app_version: None,
            }),
            overrides: Default::default(),
        }
        .game_fingerprint()
    };
    assert_ne!(split("a\0b", "c"), split("a", "b\0c"));
}

#[test]
fn the_trace_header_carries_each_fingerprint_and_the_format_version() {
    let id = identity("4.91", "NPAA00001", "base");
    assert_eq!(
        id.trace_header(),
        cellgov_trace::TraceRecord::RunIdentity {
            format_version: cellgov_trace::TRACE_FORMAT_VERSION,
            firmware: id.firmware_fingerprint(),
            game: id.game_fingerprint(),
            overrides: 0,
        }
    );
}

#[test]
fn trace_identity_reads_the_leading_header() {
    let id = identity("4.91", "NPAA00001", "base");
    let mut writer = cellgov_trace::TraceWriter::new();
    writer.record_header(&id.trace_header());
    writer.record(&cellgov_trace::TraceRecord::PpuStateHash {
        step: 0,
        pc: 0x10000,
        hash: cellgov_trace::StateHash::new(1),
    });
    let found = trace_identity(writer.bytes()).expect("header present");
    assert_eq!(found.format_version, cellgov_trace::TRACE_FORMAT_VERSION);
    assert_eq!(found.firmware, id.firmware_fingerprint());
    assert_eq!(found.game, id.game_fingerprint());
}

#[test]
fn a_headerless_stream_carries_no_identity() {
    let mut writer = cellgov_trace::TraceWriter::new();
    writer.record(&cellgov_trace::TraceRecord::PpuStateHash {
        step: 0,
        pc: 0x10000,
        hash: cellgov_trace::StateHash::new(1),
    });
    assert_eq!(trace_identity(writer.bytes()), None);
    assert_eq!(trace_identity(&[]), None);
}

#[test]
fn a_header_that_is_not_first_is_not_the_stream_identity() {
    let id = identity("4.91", "NPAA00001", "base");
    // `record_header` refuses a writer that already holds bytes, so
    // the only way to get a trailing header into a stream is to
    // append one stream's bytes to another's.
    let mut body = cellgov_trace::TraceWriter::new();
    body.record(&cellgov_trace::TraceRecord::PpuStateHash {
        step: 0,
        pc: 0x10000,
        hash: cellgov_trace::StateHash::new(1),
    });
    let mut trailing = cellgov_trace::TraceWriter::new();
    assert!(
        trailing.record_header(&id.trace_header()),
        "a fresh writer accepts the header"
    );
    let mut bytes = body.take_bytes();
    bytes.extend_from_slice(trailing.bytes());
    assert_eq!(trace_identity(&bytes), None);
}

#[test]
fn sentinel_line_round_trips() {
    let id = identity("4.91", "NPAA00001", "update:02.51");
    let line = id.render_sentinel_line().expect("render");
    let noise = format!("boot: something\n{line}\nBENCH_RESULT steps=1\n");
    assert_eq!(
        RunIdentity::parse_sentinel_lines(&noise).expect("parse"),
        Some(id)
    );
}

#[test]
fn text_with_no_sentinel_line_parses_as_no_identity() {
    assert_eq!(
        RunIdentity::parse_sentinel_lines("boot: something\n").expect("parse"),
        None
    );
}

#[test]
fn two_sentinel_lines_are_refused() {
    let line = identity("4.91", "NPAA00001", "base")
        .render_sentinel_line()
        .expect("render");
    let text = format!("{line}\n{line}\n");
    assert!(matches!(
        RunIdentity::parse_sentinel_lines(&text),
        Err(SentinelParseError::Repeated)
    ));
}

#[test]
fn a_token_that_only_starts_with_the_sentinel_is_not_a_sentinel_line() {
    let text = format!("{RUN_IDENTITY_SENTINEL}_EXTRA {{}}\n{RUN_IDENTITY_SENTINEL}\n");
    assert_eq!(
        RunIdentity::parse_sentinel_lines(&text).expect("neither line is this sentinel"),
        None
    );
}

#[test]
fn a_malformed_sentinel_payload_is_refused() {
    let text = format!("{RUN_IDENTITY_SENTINEL} not-json\n");
    assert!(matches!(
        RunIdentity::parse_sentinel_lines(&text),
        Err(SentinelParseError::Malformed { .. })
    ));
}

#[test]
fn matching_triples_produce_no_warning() {
    let id = identity("4.91", "NPAA00001", "base");
    assert!(cross_identity_warning(&id, "a", &id, "b").is_empty());
}

#[test]
fn a_firmware_difference_warns() {
    let a = identity("4.91", "NPAA00001", "base");
    let b = identity("4.93", "NPAA00001", "base");
    let lines = cross_identity_warning(&a, "a.json", &b, "b.json");
    assert!(
        lines.iter().any(|l| l.contains("cross-firmware")),
        "{lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("cross-version")),
        "the game half agrees: {lines:?}"
    );
}

#[test]
fn a_game_version_difference_warns() {
    let a = identity("4.91", "NPAA00001", "base");
    let b = identity("4.91", "NPAA00001", "update:02.51");
    let lines = cross_identity_warning(&a, "a.json", &b, "b.json");
    assert!(
        lines.iter().any(|l| l.contains("cross-version")),
        "{lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("cross-firmware")),
        "the firmware half agrees: {lines:?}"
    );
}

#[test]
fn both_halves_moving_warns_once_for_each_plus_the_verdict() {
    let a = identity("4.91", "NPAA00001", "base");
    let b = identity("4.93", "NPAA00001", "update:02.51");
    let lines = cross_identity_warning(&a, "a.json", &b, "b.json");
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(lines[0].contains("cross-firmware"), "{lines:?}");
    assert!(lines[1].contains("cross-version"), "{lines:?}");
    assert!(lines[2].contains("not a regression"), "{lines:?}");
}

#[test]
fn a_side_with_only_one_half_warns_about_the_half_it_lacks() {
    let a = identity("4.91", "NPAA00001", "base");
    let b = RunIdentity {
        firmware: a.firmware.clone(),
        game: None,
        overrides: Default::default(),
    };
    let lines = cross_identity_warning(&a, "a.json", &b, "b.json");
    assert!(
        lines.iter().any(|l| l.contains("no store entry")),
        "the firmware halves agree and only the game half is missing: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("cross-firmware")),
        "{lines:?}"
    );
}

#[test]
fn an_unidentified_side_never_warns() {
    let a = identity("4.91", "NPAA00001", "base");
    assert!(cross_identity_warning(&a, "a", &RunIdentity::default(), "b").is_empty());
    assert!(cross_identity_warning(&RunIdentity::default(), "a", &a, "b").is_empty());
}

#[test]
fn two_trace_headers_that_disagree_warn() {
    let a = identity("4.91", "NPAA00001", "base");
    let b = identity("4.93", "NPAA00001", "base");
    let lines = cross_trace_identity_warning(
        trace_identity_of(&a),
        "a.state",
        trace_identity_of(&b),
        "b.state",
    );
    assert!(
        lines[0].contains("disagree on firmware"),
        "the firmware half moved and the game half did not: {lines:?}"
    );
    assert_eq!(lines.len(), 3, "one verdict line plus one per side");
}

#[test]
fn two_trace_headers_written_under_different_formats_warn() {
    let id = identity("4.91", "NPAA00001", "base");
    let current = trace_identity_of(&id).expect("header present");
    let older = TraceIdentity {
        format_version: current.format_version - 1,
        ..current
    };
    let lines = cross_trace_identity_warning(Some(current), "a.state", Some(older), "b.state");
    assert!(
        lines.iter().any(|l| l.contains("cross-format")),
        "{lines:?}"
    );
}

#[test]
fn a_format_difference_warns_even_when_neither_side_names_a_triple() {
    let anonymous = |version| TraceIdentity {
        format_version: version,
        firmware: 0,
        game: 0,
        overrides: 0,
    };
    let lines = cross_trace_identity_warning(
        Some(anonymous(cellgov_trace::TRACE_FORMAT_VERSION)),
        "a.state",
        Some(anonymous(cellgov_trace::TRACE_FORMAT_VERSION + 1)),
        "b.state",
    );
    assert_eq!(
        lines.len(),
        1,
        "the format line, and no identity-triple line"
    );
    assert!(lines[0].contains("cross-format"), "{lines:?}");
}

#[test]
fn two_matching_trace_headers_do_not_warn() {
    let a = identity("4.91", "NPAA00001", "base");
    assert!(cross_trace_identity_warning(
        trace_identity_of(&a),
        "a.state",
        trace_identity_of(&a),
        "b.state"
    )
    .is_empty());
}

#[test]
fn a_headerless_trace_never_warns() {
    let a = identity("4.91", "NPAA00001", "base");
    assert!(
        cross_trace_identity_warning(trace_identity_of(&a), "a.state", None, "b.state").is_empty(),
        "a stream that names no identity triple makes no claim to contradict"
    );
}

#[test]
fn a_header_that_names_neither_half_never_warns() {
    let a = identity("4.91", "NPAA00001", "base");
    let unidentified = trace_identity_of(&RunIdentity::default());
    assert_eq!(
        unidentified,
        Some(TraceIdentity {
            format_version: cellgov_trace::TRACE_FORMAT_VERSION,
            firmware: 0,
            game: 0,
            overrides: 0,
        }),
        "an unidentified run still leads its stream with a header"
    );
    assert!(
        cross_trace_identity_warning(trace_identity_of(&a), "a.state", unidentified, "b.state")
            .is_empty(),
        "an all-zero header is the stream-level spelling of an unidentified run"
    );
    assert!(cross_trace_identity_warning(
        unidentified,
        "a.state",
        trace_identity_of(&a),
        "b.state"
    )
    .is_empty());
}

fn trace_identity_of(id: &RunIdentity) -> Option<TraceIdentity> {
    let mut writer = cellgov_trace::TraceWriter::new();
    writer.record_header(&id.trace_header());
    trace_identity(writer.bytes())
}

#[test]
fn the_report_prints_both_sides_even_when_they_agree() {
    let id = identity("4.91", "NPAA00001", "base");
    let lines = identity_report(&id, "a.json", &id, "b.json");
    assert!(lines.iter().any(|l| l.starts_with("a.json:")), "{lines:?}");
    assert!(lines.iter().any(|l| l.starts_with("b.json:")), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("WARN")), "{lines:?}");
}

#[test]
fn the_report_is_silent_when_neither_side_is_identified() {
    let empty = RunIdentity::default();
    assert!(identity_report(&empty, "a", &empty, "b").is_empty());
}

#[test]
fn the_report_names_an_unidentified_side() {
    let id = identity("4.91", "NPAA00001", "base");
    let lines = identity_report(&id, "a.json", &RunIdentity::default(), "b.json");
    assert!(
        lines.iter().any(|l| l.contains("(unidentified)")),
        "{lines:?}"
    );
}

/// The base keeps its own spelling; every other version is an update,
/// whatever it looks like.
#[test]
fn a_game_version_spells_the_base_bare_and_an_update_with_its_prefix() {
    assert_eq!(GameIdentity::version_of(BASE_VERSION), "base");
    assert_eq!(GameIdentity::version_of("02.51"), "update:02.51");
    assert_eq!(GameIdentity::version_of("Base"), "update:Base");
}
