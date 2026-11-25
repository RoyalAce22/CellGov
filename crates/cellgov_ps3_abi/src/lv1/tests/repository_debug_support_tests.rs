use super::*;
use crate::lv1::partition::LPAR_ID_PME;

fn debug_support() -> RepositoryKey {
    RepositoryKey {
        lpar: LPAR_ID_PME,
        n1: first_field(b"ss", 0),
        n2: field(b"param", 0),
        n3: field(b"debug", 0),
        n4: field(b"support", 0),
    }
}

/// The 4.93 kernel builds these words in r4 to r7 before its `sc`
/// [OFW-4.93 lv2_kernel a:0x8000000000050a08-0x8000000000050a48].
#[test]
fn the_served_debug_support_key_is_the_words_the_4_93_kernel_builds() {
    let served: Vec<RepositoryKey> = NODES
        .iter()
        .map(|node| node.key)
        .filter(|key| key.path() == "ss.param.debug.support")
        .collect();
    assert_eq!(served.len(), 1);
    let key = served[0];
    assert_eq!(
        (key.lpar, key.n1, key.n2, key.n3, key.n4),
        (
            1,
            0x0000_0000_7373_0000,
            0x7061_7261_6d00_0000,
            0x6465_6275_6700_0000,
            0x7375_7070_6f72_7400,
        )
    );
}

/// The values come from the console capture
/// `tests/micro/lv1_repository_debug_support/peek/cech20-cex-493`.
#[test]
fn the_4_93_debug_support_node_answers_all_ones_in_both_values_cited_to_the_lv1_capture() {
    let node = lookup(&debug_support(), "4.93").expect("a 4.93 row for ss.param.debug.support");
    assert_eq!(node.v1, 0xffff_ffff_ffff_ffff);
    assert_eq!(node.v2, SecondValue::Given(0xffff_ffff_ffff_ffff));
    assert_eq!(
        node.source,
        NodeSource::Lv1NodeCapture {
            capture: DEBUG_SUPPORT_CAPTURE,
            address: 0x0076_3d58,
        }
    );
    assert_eq!(node.source.class(), "console");
    assert_eq!(node.source.citation(), DEBUG_SUPPORT_CAPTURE);
    assert_eq!(node.source.section().as_deref(), Some("0x0000000000763d58"));
}

#[test]
fn a_floor_read_of_the_debug_support_node_is_refused() {
    for fw in ["1.50", "1.94", "3.70"] {
        assert!(lookup(&debug_support(), fw).is_none(), "{fw}");
    }
}
