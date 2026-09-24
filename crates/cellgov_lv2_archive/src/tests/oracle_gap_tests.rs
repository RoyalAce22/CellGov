//! The dispatch-table parse and the overlay's text form.

use super::*;

#[test]
fn unbound_lines_name_ordinals_and_ranges_bound_lines_do_not() {
    // The table writes each row's ordinal straight after the `//`.
    let source = "\
        null_func, //0 (0x000)\n\
        BIND_SYSC(sys_process_getpid), //1 (0x001)\n\
        uns_func, uns_func, uns_func, //2-4 unused\n\
        BIND_SYSC(a), BIND_SYSC(b), //5-6\n\
        null_func, //7words read as 7\n\
        // a comment line naming no ordinal\n\
        null_func, // 8 with a space names no ordinal\n\
        no comment here\n\
        null_func, //3 repeats a range member once\n";
    assert_eq!(
        unbound_ordinals(source),
        [0, 2, 3, 4, 7].into_iter().collect()
    );
}

#[test]
fn a_malformed_range_is_not_a_table_row() {
    assert!(unbound_ordinals("x, //3-\ny, //-4\nz, //5-6-7\n").is_empty());
}

#[test]
fn the_overlay_round_trips() {
    let ordinals: BTreeSet<u64> = [3, 14, 1024].into_iter().collect();
    let text = overlay_text("abc123", &ordinals);
    assert_eq!(text, "revision\tabc123\nordinal\n3\n14\n1024\n");
    assert_eq!(parse_overlay(&text), Ok(ordinals));
    assert_eq!(
        parse_overlay(&overlay_text("r", &BTreeSet::new())),
        Ok(BTreeSet::new())
    );
}

#[test]
fn an_overlay_saved_with_crlf_line_endings_lists_its_ordinals() {
    let text = "revision\tabc123\r\nordinal\r\n3\r\n14\r\n";
    assert_eq!(parse_overlay(text), Ok([3, 14].into_iter().collect()));
}

#[test]
fn a_malformed_row_is_refused_by_line_and_text() {
    let text = "revision\tabc123\nordinal\n14\n1 5\n3\n";
    assert_eq!(
        parse_overlay(text),
        Err(OverlayParseError::MalformedRow {
            line: 4,
            row: "1 5".to_string(),
        })
    );
    assert_eq!(
        parse_overlay("revision\tabc123\nordinal\n14\n\n3\n"),
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
        parse_overlay("revision\tabc123\n14\n"),
        Err(OverlayParseError::MissingColumnHeader)
    );
}
