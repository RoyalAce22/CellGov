//! The stop list: the peeks it refuses, and its agreement with the
//! prose ledger.

use super::*;

/// A ledger whose one peek row is `call`, with `status`.
fn ledger_with(call: &str, status: &str) -> String {
    format!(
        "# Console crash ledger\n\n| Call | Status | Evidence |\n| --- | --- | --- |\n\
         | `sc 801` open under `/dev_flash` | suspected | a run crashed |\n\
         | {call} | {status} | a run crashed |\n"
    )
}

const ONE_RANGE_LEDGER_CALL: &str = "webMAN `peek.lv2` of 0x1000 to 0x1FFF, a window";

const ONE_RANGE_LIST: &str = "space\tfirst\tlast\tstatus\nlv2\t0x1000\t0x1fff\tsuspected\n";

fn one_range() -> StopList {
    StopList::parse(
        ONE_RANGE_LIST,
        &ledger_with(ONE_RANGE_LEDGER_CALL, "suspected"),
    )
    .expect("agrees")
}

#[test]
fn a_peek_whose_page_overlaps_a_listed_range_is_refused() {
    let list = one_range();
    let cases: &[(&str, Space, u64, u64, bool)] = &[
        (
            "the page below, ending just short",
            Space::Lv2,
            0x0e00,
            0x0fff,
            true,
        ),
        (
            "the page above, starting just past",
            Space::Lv2,
            0x2000,
            0x21ff,
            true,
        ),
        (
            "a page over the first address",
            Space::Lv2,
            0x0f00,
            0x10ff,
            false,
        ),
        (
            "a page over the last address",
            Space::Lv2,
            0x1f00,
            0x20ff,
            false,
        ),
        (
            "a page sharing only the last byte",
            Space::Lv2,
            0x1fff,
            0x21fe,
            false,
        ),
        (
            "a page sharing only the first byte",
            Space::Lv2,
            0x0e01,
            0x1000,
            false,
        ),
        ("a page inside", Space::Lv2, 0x1200, 0x13ff, false),
        (
            "a page around the whole range",
            Space::Lv2,
            0x0000,
            0xffff,
            false,
        ),
        (
            "the same addresses in the other space",
            Space::Lv1,
            0x1000,
            0x11ff,
            true,
        ),
    ];
    for (name, space, first, last, admitted) in cases {
        match list.admit(*space, *first, *last) {
            Ok(()) => assert!(admitted, "{name}: admitted"),
            Err(LedgerError::Listed { listed, .. }) => {
                assert!(!admitted, "{name}: refused");
                assert_eq!(listed, list.ranges()[0], "{name}");
            }
            Err(other) => panic!("{name}: {other}"),
        }
    }
}

#[test]
fn the_committed_ledger_and_stop_list_agree_and_refuse_the_page_table_window() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let list = StopList::load(&root).expect("the committed ledger and stop list agree");
    assert_eq!(list.ranges().len(), 3);
    let window = StopRange {
        space: Space::Lv2,
        first: 0x8000_0000_0f00_0000,
        last: 0x8000_0000_0f03_ffff,
        status: Status::Suspected,
    };
    assert_eq!(list.ranges().first(), Some(&window));
    let cases: &[(&str, Space, u64, bool)] = &[
        (
            "the page the crash read",
            Space::Lv2,
            0x8000_0000_0f00_0000,
            false,
        ),
        (
            "the window's last page",
            Space::Lv2,
            0x8000_0000_0f03_fe00,
            false,
        ),
        (
            "the page after the window",
            Space::Lv2,
            0x8000_0000_0f04_0000,
            true,
        ),
        (
            "the page before the window",
            Space::Lv2,
            0x8000_0000_0eff_fe00,
            true,
        ),
        ("the lv1 console gate page", Space::Lv1, 0x0037_4000, true),
        (
            "the kernel copy page",
            Space::Lv2,
            0x8000_0000_0035_4c00,
            true,
        ),
    ];
    for (name, space, page, admitted) in cases {
        match list.admit(*space, *page, page + 0x1ff) {
            Ok(()) => assert!(admitted, "{name}: admitted"),
            Err(LedgerError::Listed { listed, .. }) => {
                assert!(!admitted, "{name}: refused");
                assert_eq!(listed, window, "{name}");
            }
            Err(other) => panic!("{name}: {other}"),
        }
    }
}

#[test]
fn a_stop_list_that_does_not_match_the_ledger_is_refused() {
    let agreeing = ledger_with(ONE_RANGE_LEDGER_CALL, "suspected");
    let cases: &[(&str, &str, String)] = &[
        (
            "a status the ledger does not give",
            "space\tfirst\tlast\tstatus\nlv2\t0x1000\t0x1fff\tconfirmed\n",
            agreeing.clone(),
        ),
        (
            "a range end the ledger does not give",
            "space\tfirst\tlast\tstatus\nlv2\t0x1000\t0x11ff\tsuspected\n",
            agreeing.clone(),
        ),
        (
            "a space the ledger does not give",
            "space\tfirst\tlast\tstatus\nlv1\t0x1000\t0x1fff\tsuspected\n",
            agreeing.clone(),
        ),
        (
            "no range for the ledger's peek row",
            "space\tfirst\tlast\tstatus\n",
            agreeing.clone(),
        ),
        (
            "a range the ledger has no row for",
            ONE_RANGE_LIST,
            "| Call | Status | Evidence |\n| --- | --- | --- |\n".to_string(),
        ),
    ];
    for (name, list, ledger) in cases {
        match StopList::parse(list, ledger) {
            Err(LedgerError::Disagree { .. }) => {}
            other => panic!("{name}: {other:?}"),
        }
    }
}

#[test]
fn a_line_either_file_cannot_read_is_named() {
    let agreeing = ledger_with(ONE_RANGE_LEDGER_CALL, "suspected");
    let cases: &[(&str, &str, String, &str, usize)] = &[
        (
            "a stop list with no header",
            "lv2\t0x1000\t0x1fff\tsuspected\n",
            agreeing.clone(),
            STOP_LIST,
            1,
        ),
        (
            "a stop list row of three fields",
            "space\tfirst\tlast\tstatus\nlv2\t0x1000\tsuspected\n",
            agreeing.clone(),
            STOP_LIST,
            2,
        ),
        (
            "a stop list row whose first is above its last",
            "space\tfirst\tlast\tstatus\nlv2\t0x2000\t0x1fff\tsuspected\n",
            agreeing.clone(),
            STOP_LIST,
            2,
        ),
        (
            "a stop list address that is not 0x hex",
            "space\tfirst\tlast\tstatus\nlv2\t1000\t0x1fff\tsuspected\n",
            agreeing.clone(),
            STOP_LIST,
            2,
        ),
        (
            "a ledger peek row with no range",
            ONE_RANGE_LIST,
            ledger_with("webMAN `peek.lv2` of the page table", "suspected"),
            LEDGER,
            6,
        ),
        (
            "a ledger peek row with a status it does not know",
            ONE_RANGE_LIST,
            ledger_with(ONE_RANGE_LEDGER_CALL, "likely"),
            LEDGER,
            6,
        ),
    ];
    for (name, list, ledger, file, line) in cases {
        match StopList::parse(list, ledger) {
            Err(LedgerError::Malformed {
                file: f, line: l, ..
            }) => assert_eq!((f, l), (*file, *line), "{name}"),
            other => panic!("{name}: {other:?}"),
        }
    }
}
