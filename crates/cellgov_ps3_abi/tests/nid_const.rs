//! End-to-end smoke test for the `nid_const!` macro.
//!
//! A wrong literal here would not fail at test time -- it would fail
//! `cargo build`. The runtime asserts double-check that the pinned
//! constants take the values the macro pinned them to.

use cellgov_ps3_abi::nid_const;

mod cell_spurs {
    cellgov_ps3_abi::nid_const!(INITIALIZE = 0xacfc_8dbc, "cellSpursInitialize");
    cellgov_ps3_abi::nid_const!(FINALIZE = 0xca4c_4600, "cellSpursFinalize");
}

mod cell_gcm_sys {
    cellgov_ps3_abi::nid_const!(_CELLGCM_INIT_BODY = 0x15ba_e46b, "_cellGcmInitBody");
}

nid_const!(SYS_LWMUTEX_CREATE = 0x2f85_c0ef, "sys_lwmutex_create");

#[test]
fn macro_pins_known_anchor_values() {
    assert_eq!(cell_spurs::INITIALIZE, 0xacfc_8dbc);
    assert_eq!(cell_spurs::FINALIZE, 0xca4c_4600);
    assert_eq!(cell_gcm_sys::_CELLGCM_INIT_BODY, 0x15ba_e46b);
    assert_eq!(SYS_LWMUTEX_CREATE, 0x2f85_c0ef);
}
