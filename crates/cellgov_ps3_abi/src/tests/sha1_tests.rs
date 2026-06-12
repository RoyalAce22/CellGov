//! nid_sha1 anchors pinning known PS3 export NIDs at runtime and compile time.

use super::nid_sha1;

#[test]
fn anchor_cell_spurs_initialize() {
    assert_eq!(nid_sha1("cellSpursInitialize"), 0xacfc_8dbc);
}

#[test]
fn anchor_cell_spurs_finalize() {
    assert_eq!(nid_sha1("cellSpursFinalize"), 0xca4c_4600);
}

#[test]
fn anchor_cell_spurs_add_workload() {
    assert_eq!(nid_sha1("cellSpursAddWorkload"), 0x6972_6aa2);
}

#[test]
fn anchor_cell_gcm_init_body() {
    assert_eq!(nid_sha1("_cellGcmInitBody"), 0x15ba_e46b);
}

#[test]
fn anchor_sys_lwmutex_create() {
    assert_eq!(nid_sha1("sys_lwmutex_create"), 0x2f85_c0ef);
}

const _COMPILE_TIME_ANCHOR: () = {
    assert!(nid_sha1("cellSpursInitialize") == 0xacfc_8dbc);
    assert!(nid_sha1("cellSpursFinalize") == 0xca4c_4600);
};
