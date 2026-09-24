//! The oracle-gap overlay's reader against the shape its writer emits.

use super::*;

#[test]
fn an_overlay_in_the_written_shape_lists_its_ordinals() {
    let text = format!("{REVISION_KEY}\tabc123\n{ORDINAL_HEADER}\n14\n3\n14\n");
    assert_eq!(parse_overlay(&text), Ok([3, 14].into_iter().collect()));
}

#[test]
fn an_overlay_saved_with_crlf_line_endings_lists_its_ordinals() {
    let text = format!("{REVISION_KEY}\tabc123\r\n{ORDINAL_HEADER}\r\n3\r\n14\r\n");
    assert_eq!(parse_overlay(&text), Ok([3, 14].into_iter().collect()));
}

#[test]
fn an_overlay_with_no_rows_lists_nothing() {
    let text = format!("{REVISION_KEY}\tabc123\n{ORDINAL_HEADER}\n");
    assert_eq!(parse_overlay(&text), Ok(Default::default()));
}

#[test]
fn a_malformed_row_is_refused_by_line_and_text() {
    let text = format!("{REVISION_KEY}\tabc123\n{ORDINAL_HEADER}\n14\n1 5\n3\n");
    assert_eq!(
        parse_overlay(&text),
        Err(OverlayParseError::MalformedRow {
            line: 4,
            row: "1 5".to_string(),
        })
    );
}

#[test]
fn a_blank_row_is_refused() {
    let text = format!("{REVISION_KEY}\tabc123\n{ORDINAL_HEADER}\n14\n\n3\n");
    assert_eq!(
        parse_overlay(&text),
        Err(OverlayParseError::MalformedRow {
            line: 4,
            row: String::new(),
        })
    );
}

#[test]
fn an_overlay_missing_either_header_line_is_refused() {
    assert_eq!(
        parse_overlay("14\n3\n"),
        Err(OverlayParseError::MissingRevision)
    );
    assert_eq!(parse_overlay(""), Err(OverlayParseError::MissingRevision));
    assert_eq!(
        parse_overlay(&format!("{REVISION_KEY}\tabc123\n14\n")),
        Err(OverlayParseError::MissingColumnHeader)
    );
}
