use super::*;
use crate::lv1::partition::{LPAR_ID_CURRENT, LPAR_ID_GAME_OS};

fn bi(second: &[u8], n3: u64, lpar: u64) -> RepositoryKey {
    RepositoryKey {
        lpar,
        n1: first_field(b"bi", 0),
        n2: field(second, 0),
        n3,
        n4: 0,
    }
}

/// The raw words are those of the repository dump
/// [PS3DEVWIKI-REPO s:3.41_GameOS], `00 00 00 00 62 69 00 00` and
/// `70 75 6e 00 00 00 00 00`.
#[test]
fn the_key_words_pack_text_from_the_high_byte() {
    assert_eq!(first_field(b"bi", 0), 0x6269_0000);
    assert_eq!(field(b"pun", 0), 0x7075_6e00_0000_0000);
    assert_eq!(field(b"rgntotal", 0), 0x7267_6e74_6f74_616c);
}

/// Linux `make_field` and `make_first_field` copy at most 8 bytes of the
/// text.
#[test]
fn a_key_word_keeps_only_the_first_8_bytes_of_its_text() {
    assert_eq!(field(b"rgntotalX", 0), 0x7267_6e74_6f74_616c);
    assert_eq!(first_field(b"rgntotalX", 0), 0x7267_6e74);
}

#[test]
fn an_index_adds_to_the_key_word() {
    assert_eq!(first_field(b"bus", 3), 0x6275_7303);
    assert_eq!(field(b"pu", 1), 0x7075_0000_0000_0001);
}

/// A firmware that CellGov serves the psdevwiki dump's values to.
const DUMP_FW: &str = "1.50";

#[test]
fn the_nodes_ascend_by_key_and_then_by_firmware() {
    assert_eq!(NODES.len(), 41);
    for pair in NODES.windows(2) {
        assert!(
            (pair[0].key, pair[0].firmware) < (pair[1].key, pair[1].firmware),
            "{pair:?}"
        );
    }
}

#[test]
fn each_node_names_its_firmware_ascending_and_no_two_nodes_of_a_key_share_one() {
    assert!(!NODES.is_empty());
    for node in NODES {
        assert!(!node.firmware.is_empty(), "{} names no firmware", node.key);
        assert!(
            node.firmware.windows(2).all(|pair| pair[0] < pair[1]),
            "{} names {:?}",
            node.key,
            node.firmware
        );
    }
    for (at, node) in NODES.iter().enumerate() {
        for other in NODES[at + 1..].iter().filter(|other| other.key == node.key) {
            let shared: Vec<&&str> = node
                .firmware
                .iter()
                .filter(|fw| other.firmware.contains(fw))
                .collect();
            assert!(shared.is_empty(), "two {} nodes name {shared:?}", node.key);
        }
    }
}

#[test]
fn lookup_finds_no_node_at_a_firmware_the_node_does_not_name() {
    let pun = bi(b"pun", 0, LPAR_ID_GAME_OS);
    assert_eq!(lookup(&pun, DUMP_FW).map(|n| n.v1), Some(1));
    assert!(lookup(&pun, "3.41").is_none());
    assert!(lookup(&pun, "").is_none());
}

/// [`NODES`] holds `bi.pun` = 1 and `bi.pu` = 1, from the repository
/// dump [PS3DEVWIKI-REPO s:3.41_GameOS].
#[test]
fn lookup_answers_the_game_os_pu_count_and_pu_id() {
    let value = |k: RepositoryKey| lookup(&k, DUMP_FW).map(|n| (n.v1, n.v2));
    let given = |v1| Some((v1, SecondValue::Given(0)));
    assert_eq!(value(bi(b"pun", 0, LPAR_ID_GAME_OS)), given(1));
    assert_eq!(value(bi(b"pu", 0, LPAR_ID_GAME_OS)), given(1));
}

/// The repository dump [PS3DEVWIKI-REPO s:3.41_GameOS] gives
/// `bi.pu.1.rm_addr` = 0 and `bi.pu.1.rm_size` = 0x01000000. PU id 1 is
/// the third key word, as Linux `ps3_repository_read_rm_size` passes the
/// PPE id.
#[test]
fn lookup_answers_the_real_mode_base_and_size_of_pu_1() {
    let rm = |name: &[u8]| RepositoryKey {
        n4: field(name, 0),
        ..bi(b"pu", 1, LPAR_ID_GAME_OS)
    };
    let value = |k: RepositoryKey| lookup(&k, DUMP_FW).map(|n| (n.v1, n.v2));
    let given = |v1| Some((v1, SecondValue::Given(0)));
    assert_eq!(value(rm(b"rm_addr")), given(0));
    assert_eq!(value(rm(b"rm_size")), given(0x0100_0000));
    assert_eq!(
        value(RepositoryKey {
            n3: 0,
            ..rm(b"rm_size")
        }),
        None
    );
}

#[test]
fn every_node_cites_the_3_41_game_os_dump_by_its_alias_or_a_4_93_capture() {
    let mut dump = 0;
    let mut capture = 0;
    let mut lv1_capture = 0;
    for node in NODES {
        match node.source {
            NodeSource::PsdevwikiDump { .. } => {
                assert_eq!(node.source.citation(), "PS3DEVWIKI-REPO");
                assert_eq!(node.source.section().as_deref(), Some("3.41_GameOS"));
                dump += 1;
            }
            NodeSource::ConsoleCapture { address, .. } => {
                assert_eq!(node.source.citation(), BOOT_RECORD_CAPTURE);
                assert_eq!(node.source.section(), Some(format!("{address:#018x}")));
                assert_eq!(node.firmware, ["4.93"], "{}", node.key);
                capture += 1;
            }
            NodeSource::Lv1NodeCapture { address, .. } => {
                assert!(
                    [
                        DEBUG_SUPPORT_CAPTURE,
                        FLASH_EXT_CAPTURE,
                        HW_CONFIG_CAPTURE,
                        LIST_CAPTURE,
                        BUS_CAPTURE
                    ]
                    .contains(&node.source.citation()),
                    "{}",
                    node.key
                );
                assert_eq!(node.source.section(), Some(format!("{address:#018x}")));
                assert_eq!(node.firmware, ["4.93"], "{}", node.key);
                lv1_capture += 1;
            }
        }
    }
    assert_eq!((dump, capture, lv1_capture), (4, 7, 30));
}

#[test]
fn lookup_compares_every_raw_word_and_the_partition() {
    let mut first = bi(b"pun", 0, LPAR_ID_GAME_OS);
    first.n1 = first_field(b"bi", 1);
    assert!(lookup(&first, DUMP_FW).is_none());
    let mut second = bi(b"pun", 0, LPAR_ID_GAME_OS);
    second.n2 = field(b"pun", 1);
    assert!(lookup(&second, DUMP_FW).is_none());
    assert!(lookup(&bi(b"pu", 1, LPAR_ID_GAME_OS), DUMP_FW).is_none());
    assert!(lookup(&bi(b"pun", 0, LPAR_ID_CURRENT), DUMP_FW).is_none());
    let mut fourth = bi(b"pun", 0, LPAR_ID_GAME_OS);
    fourth.n4 = 1;
    assert!(lookup(&fourth, DUMP_FW).is_none());
}
