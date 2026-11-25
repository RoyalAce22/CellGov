use super::*;

/// The 4.93 console capture reads back only the doubleword the kernel
/// stores from each read's first output. The README of
/// [`BOOT_RECORD_CAPTURE`] names the read sites.
#[test]
fn a_console_capture_node_marks_its_second_value_unobserved_and_a_dump_node_gives_it() {
    let mut capture = 0;
    for node in NODES {
        match (node.source, node.v2) {
            (NodeSource::ConsoleCapture { .. }, SecondValue::Unobserved { discarded_by }) => {
                assert_eq!(discarded_by.len(), 1, "{}", node.key);
                assert_eq!(discarded_by[0].sc, READ_WRAPPER_SC_4_93, "{}", node.key);
                capture += 1;
            }
            (NodeSource::PsdevwikiDump { .. }, SecondValue::Given(0)) => {}
            (source, v2) => panic!("{} has source {source:?} and v2 {v2:?}", node.key),
        }
    }
    assert_eq!(capture, 7);
}

#[test]
fn no_two_unobserved_nodes_share_a_read_site() {
    let sites: Vec<ReadSite> = NODES
        .iter()
        .filter_map(|node| match node.v2 {
            SecondValue::Unobserved { discarded_by } => Some(discarded_by),
            SecondValue::Given(_) => None,
        })
        .flatten()
        .copied()
        .collect();
    assert_eq!(sites.len(), 7);
    for (at, site) in sites.iter().enumerate() {
        assert!(!sites[at + 1..].contains(site), "{site:?} repeats");
    }
}
