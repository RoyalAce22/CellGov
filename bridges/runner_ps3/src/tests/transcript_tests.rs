//! The transcript's numbering and line shape, and the redactor that
//! keeps console identifiers out of it.

use super::*;

/// A status page in the shape webMAN serves, with made-up identifiers.
const STATUS_PAGE: &str = concat!(
    "<tr><td>Firmware</td><td>4.93 CEX Cobra 8.5</td></tr>",
    "<tr><td>IDPS</td><td><a href=\"/idps.ps3\">0000000100850009141C2F2E3D4A5B6C</a></td></tr>",
    "<tr><td>PSID:</td><td>7A6B5C4D3E2F1A0B9C8D7E6F5A4B3C2D</td></tr>",
    "<tr><td>MAC</td><td>00:1F:A7:12:34:56</td></tr>",
    "<tr><td>LAN</td><td>00-1f-a7-65-43-21</td></tr>",
);

#[test]
fn each_line_carries_its_number_and_direction() {
    let mut transcript = Transcript::new();
    transcript.request("GET /cpursx.ps3");
    transcript.reply("200 (5 bytes)");
    transcript.decision("console is 4.93 CEX");
    assert_eq!(
        transcript.lines(),
        [
            "#0001 > GET /cpursx.ps3",
            "#0002 < 200 (5 bytes)",
            "#0003 = console is 4.93 CEX",
        ]
    );
    assert_eq!(
        transcript.render(),
        "#0001 > GET /cpursx.ps3\n#0002 < 200 (5 bytes)\n#0003 = console is 4.93 CEX\n"
    );
    assert_eq!(Transcript::new().render(), "");
}

#[test]
fn a_status_page_leaves_no_identifier_in_the_transcript() {
    let mut transcript = Transcript::new();
    transcript.reply(STATUS_PAGE);
    let stored = transcript.render();
    for identifier in [
        "0000000100850009141C2F2E3D4A5B6C",
        "7A6B5C4D3E2F1A0B9C8D7E6F5A4B3C2D",
        "00:1F:A7:12:34:56",
        "00-1f-a7-65-43-21",
    ] {
        assert!(
            !stored.contains(identifier),
            "{identifier} survived: {stored}"
        );
    }
    assert_eq!(stored.matches(MASKED_ID).count(), 2, "{stored}");
    assert_eq!(stored.matches(MASKED_MAC).count(), 2, "{stored}");
    assert!(stored.contains("4.93 CEX Cobra 8.5"), "{stored}");
}

#[test]
fn identifiers_in_a_row_after_both_labels_are_all_masked() {
    let page = concat!(
        "<tr><th>IDPS</th><th>PSID</th></tr>",
        "<tr><td>0000000100850009141C2F2E3D4A5B6C</td>",
        "<td>7A6B5C4D3E2F1A0B9C8D7E6F5A4B3C2D</td></tr>",
    );
    let stored = redact(page);
    assert_eq!(stored.matches(MASKED_ID).count(), 2, "{stored}");
    assert!(!stored.contains("7A6B5C4D3E2F1A0B"), "{stored}");
}

#[test]
fn a_hash_with_no_identifier_label_before_it_is_kept() {
    let sha = "a".repeat(64);
    assert_eq!(redact(&format!("frame {sha}")), format!("frame {sha}"));
    assert_eq!(redact("IDPS unavailable"), "IDPS unavailable");
}

#[test]
fn a_mac_shape_inside_a_longer_token_is_kept() {
    assert_eq!(redact("x00:1F:A7:12:34:56"), "x00:1F:A7:12:34:56");
    assert_eq!(redact("00:1F:A7:12:34:567"), "00:1F:A7:12:34:567");
    assert_eq!(redact("00:1F-A7:12:34:56"), "00:1F-A7:12:34:56");
    assert_eq!(redact("[00:1F:A7:12:34:56]"), "[<mac>]");
}

#[test]
fn a_control_character_cannot_start_a_second_line() {
    let mut transcript = Transcript::new();
    transcript.reply("220-webMAN\r\n#0099 < forged");
    assert_eq!(transcript.lines(), ["#0001 < 220-webMAN  #0099 < forged"]);
    assert_eq!(transcript.render().lines().count(), 1);
}

#[test]
fn the_transcript_lands_under_the_committed_name() {
    let dir = cellgov_testkit::scratch::scratch();
    let mut transcript = Transcript::new();
    transcript.request("GET /cpursx.ps3");
    transcript.write(&dir).expect("write");
    assert_eq!(
        std::fs::read_to_string(dir.join(TRANSCRIPT_FILE)).expect("read"),
        "#0001 > GET /cpursx.ps3\n"
    );
}
