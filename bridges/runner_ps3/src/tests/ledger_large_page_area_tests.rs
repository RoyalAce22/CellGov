use super::*;
use crate::peek::PAGE;

#[test]
fn the_committed_stop_list_refuses_the_kernel_s_1_mb_page_area() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let list = StopList::load(&root).expect("the committed ledger and stop list agree");
    let area = StopRange {
        space: Space::Lv2,
        first: 0x8000_0003_0000_0000,
        last: 0x8000_0003_ffff_ffff,
        status: Status::Suspected,
    };
    assert_eq!(list.ranges().get(2), Some(&area));
    let cases: &[(&str, u64, bool)] = &[
        ("the area's first page", 0x8000_0003_0000_0000, false),
        ("the page the crash read", 0x8000_0003_0007_b200, false),
        ("the area's last page", 0x8000_0003_ffff_fe00, false),
        ("the page before the area", 0x8000_0002_ffff_fe00, true),
        ("the page after the area", 0x8000_0004_0000_0000, true),
    ];
    for (name, page, admitted) in cases {
        match list.admit(Space::Lv2, *page, page + (PAGE - 1)) {
            Ok(()) => assert!(admitted, "{name}: admitted"),
            Err(LedgerError::Listed { listed, .. }) => {
                assert!(!admitted, "{name}: refused");
                assert_eq!(listed, area, "{name}");
            }
            Err(other) => panic!("{name}: {other}"),
        }
    }
}
