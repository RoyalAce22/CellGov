//! The committed stop list's row for the kernel's window on the logical
//! SPE shadow pages.

use super::*;
use crate::peek::PAGE;

#[test]
fn the_committed_stop_list_refuses_the_logical_spe_shadow_page_window() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let list = StopList::load(&root).expect("the committed ledger and stop list agree");
    let window = StopRange {
        space: Space::Lv2,
        first: 0x8000_0000_d000_0000,
        last: 0x8000_0000_d00f_ffff,
        status: Status::Suspected,
    };
    assert_eq!(list.ranges().get(1), Some(&window));
    let cases: &[(&str, u64, bool)] = &[
        ("the window's first page", 0x8000_0000_d000_0000, false),
        ("the page the crash read", 0x8000_0000_d00f_f000, false),
        ("the crash's word at 0xF00", 0x8000_0000_d00f_fe00, false),
        ("the page after the window", 0x8000_0000_d010_0000, true),
        ("the page before the window", 0x8000_0000_cfff_fe00, true),
    ];
    for (name, page, admitted) in cases {
        match list.admit(Space::Lv2, *page, page + (PAGE - 1)) {
            Ok(()) => assert!(admitted, "{name}: admitted"),
            Err(LedgerError::Listed { listed, .. }) => {
                assert!(!admitted, "{name}: refused");
                assert_eq!(listed, window, "{name}");
            }
            Err(other) => panic!("{name}: {other}"),
        }
    }
}
