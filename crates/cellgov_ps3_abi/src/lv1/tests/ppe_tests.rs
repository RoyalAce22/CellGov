use super::*;
use crate::lv1::boot_record_capture::captured_doubleword;
use crate::lv1::call::{self, CallSource, GET_LOGICAL_PPE_ID};
use crate::lv1::repository::BOOT_RECORD_4_93;
use crate::lv1::table::{table_entry, TableEntry};

/// The offset of `bi.pu` in the 4.93 boot record. The README of the
/// capture gives the offsets.
const BOOT_RECORD_PU: u64 = 0x18;
/// The offset of `bi.pu.<id>.rm_size` in the 4.93 boot record.
const BOOT_RECORD_RM_SIZE: u64 = 0x38;

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 69.
#[test]
fn get_logical_ppe_id_carries_its_header_number_name_and_counts() {
    assert_eq!(GET_LOGICAL_PPE_ID, 69);
    let row = call::lookup(GET_LOGICAL_PPE_ID).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(row, Some(("get_logical_ppe_id", 0, 1, CallSource::Linux)));
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_69() {
    assert_eq!(LOGICAL_PPE_ID_HANDLERS.len(), 1);
    let handler = logical_ppe_id_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x2E8F38);
    assert_eq!(
        table_entry("4.93", GET_LOGICAL_PPE_ID),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    assert_eq!(logical_ppe_id_handler("3.70"), None);
    assert_eq!(logical_ppe_id_handler("4.92"), None);
    assert_eq!(logical_ppe_id_handler(""), None);
}

/// Linux `make_first_field("bi", 0)` and `make_field("pu", 0)`.
#[test]
fn the_pu_id_node_is_bi_pu_at_index_0_of_the_game_os_partition() {
    assert_eq!(GAME_OS_PU_ID_NODE.lpar, 2);
    assert_eq!(GAME_OS_PU_ID_NODE.n1, 0x6269_0000);
    assert_eq!(GAME_OS_PU_ID_NODE.n2, 0x7075_0000_0000_0000);
    assert_eq!((GAME_OS_PU_ID_NODE.n3, GAME_OS_PU_ID_NODE.n4), (0, 0));
}

/// At 4.93 the source is the boot record capture. Before 4.93 it is the
/// repository dump [PS3DEVWIKI-REPO s:3.41_GameOS], which gives 1.
#[test]
fn the_game_os_logical_ppe_id_is_bi_pu_where_a_source_gives_it() {
    assert_eq!(
        game_os_logical_ppe_id("4.93"),
        Some(captured_doubleword(BOOT_RECORD_4_93 + BOOT_RECORD_PU))
    );
    assert_eq!(game_os_logical_ppe_id("3.70"), Some(1));
    assert_eq!(game_os_logical_ppe_id("4.92"), None);
    assert_eq!(game_os_logical_ppe_id(""), None);
}

/// Linux `ps3_repository_read_mm_info` reads `bi.pu.<id>.rm_size` with
/// the logical PPE id, so the repository holds that node for the id.
#[test]
fn the_repository_holds_the_real_mode_size_of_the_4_93_logical_ppe() {
    let id = game_os_logical_ppe_id("4.93").expect("the 4.93 id");
    let rm_size = RepositoryKey {
        n3: id,
        n4: field(b"rm_size", 0),
        ..GAME_OS_PU_ID_NODE
    };
    assert_eq!(
        lookup(&rm_size, "4.93").map(|node| node.v1),
        Some(captured_doubleword(BOOT_RECORD_4_93 + BOOT_RECORD_RM_SIZE))
    );
}
