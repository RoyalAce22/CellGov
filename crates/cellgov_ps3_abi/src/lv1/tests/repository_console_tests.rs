use super::*;
use crate::hw::ppu::CELL_PPU_TIMEBASE_HZ;
use crate::lv1::boot_record_capture::captured_doubleword;
use crate::lv1::partition::{LPAR_ID_GAME_OS, LPAR_ID_PME};

const FLOORS: [&str; 3] = ["1.50", "1.94", "3.70"];

fn key(lpar: u64, n: [u64; 4]) -> RepositoryKey {
    RepositoryKey {
        lpar,
        n1: n[0],
        n2: n[1],
        n3: n[2],
        n4: n[3],
    }
}

fn rgntotal() -> RepositoryKey {
    key(
        LPAR_ID_GAME_OS,
        [first_field(b"bi", 0), field(b"rgntotal", 0), 0, 0],
    )
}

fn plat_id() -> RepositoryKey {
    key(
        LPAR_ID_PME,
        [first_field(b"plat", 0), field(b"id", 0), 0, 0],
    )
}

fn be_0_clock() -> RepositoryKey {
    key(
        LPAR_ID_PME,
        [first_field(b"be", 0), 0, field(b"clock", 0), 0],
    )
}

fn rm(name: &[u8]) -> RepositoryKey {
    key(
        LPAR_ID_GAME_OS,
        [first_field(b"bi", 0), field(b"pu", 0), 1, field(name, 0)],
    )
}

fn rm_size() -> RepositoryKey {
    rm(b"rm_size")
}

fn bi(second: &[u8]) -> RepositoryKey {
    key(
        LPAR_ID_GAME_OS,
        [first_field(b"bi", 0), field(second, 0), 0, 0],
    )
}

/// The boot record offsets come from the console capture
/// `tests/micro/lv2_repository_boot_nodes/ps3/cech20-cex-493`. The
/// values are the doublewords its frame holds at those offsets.
#[test]
fn the_4_93_boot_nodes_answer_the_captured_values_cited_to_the_capture() {
    let expected = [
        (bi(b"pun"), 0x10),
        (bi(b"pu"), 0x18),
        (rm(b"rm_addr"), 0x30),
        (rgntotal(), 0x68),
        (rm_size(), 0x38),
        (plat_id(), 0xa8),
        (be_0_clock(), 0xb0),
    ];
    for (key, offset) in expected {
        let node = lookup(&key, "4.93").unwrap_or_else(|| panic!("no 4.93 row for {key}"));
        assert_eq!(
            node.v1,
            captured_doubleword(BOOT_RECORD_4_93 + offset),
            "{key}"
        );
        assert_eq!(
            node.source,
            NodeSource::ConsoleCapture {
                capture: BOOT_RECORD_CAPTURE,
                address: BOOT_RECORD_4_93 + offset,
            },
            "{key}"
        );
        assert_eq!(node.source.class(), "console");
        assert_eq!(node.source.citation(), BOOT_RECORD_CAPTURE);
    }
}

#[test]
fn a_floor_read_of_a_node_only_the_4_93_capture_gives_is_refused() {
    for key in [rgntotal(), plat_id(), be_0_clock()] {
        for fw in FLOORS {
            assert!(lookup(&key, fw).is_none(), "{key} at {fw}");
        }
    }
}

/// The repository dump [PS3DEVWIKI-REPO s:3.41_GameOS] gives
/// `bi.pu.1.rm_size` = 0x01000000; the 4.93 console capture gives
/// the doubleword at offset 0x38 of the boot record.
#[test]
fn the_floors_keep_the_dump_real_mode_size_and_4_93_takes_the_captured_one() {
    for fw in FLOORS {
        let node = lookup(&rm_size(), fw).unwrap_or_else(|| panic!("no rm_size at {fw}"));
        assert_eq!((node.v1, node.source.class()), (0x0100_0000, "psdevwiki"));
    }
    let node = lookup(&rm_size(), "4.93").expect("rm_size at 4.93");
    let captured = captured_doubleword(BOOT_RECORD_4_93 + 0x38);
    assert_ne!(captured, 0x0100_0000);
    assert_eq!((node.v1, node.source.class()), (captured, "console"));
}

/// The dump [PS3DEVWIKI-REPO s:3.41_GameOS] and the 4.93 console capture
/// give the same values for these three nodes.
#[test]
fn the_pu_count_pu_id_and_rm_base_keep_one_value_cited_to_the_dump_then_the_4_93_capture() {
    for key in [bi(b"pun"), bi(b"pu"), rm(b"rm_addr")] {
        for fw in FLOORS {
            let node = lookup(&key, fw).unwrap_or_else(|| panic!("no {key} at {fw}"));
            assert_eq!(node.source, GAME_OS_341_DUMP, "{key} at {fw}");
        }
        let node = lookup(&key, "4.93").unwrap_or_else(|| panic!("no {key} at 4.93"));
        assert_eq!(node.source.class(), "console", "{key} at 4.93");
        assert_eq!(lookup(&key, "3.70").map(|n| n.v1), Some(node.v1), "{key}");
    }
}

#[test]
fn the_captured_time_base_frequency_is_the_cell_time_base_frequency() {
    let node = lookup(&be_0_clock(), "4.93").expect("be.0.clock at 4.93");
    assert_eq!(node.v1, CELL_PPU_TIMEBASE_HZ);
}

#[test]
fn the_be_clock_key_keeps_its_inner_0_word_and_plat_id_drops_its_trailing_ones() {
    assert_eq!(be_0_clock().path(), "be.0.clock");
    assert_eq!(plat_id().path(), "plat.id");
}
