//! FS open-flag bit values and the TTY-sink path pre-registration contract.

use cellgov_ps3_abi::sys_fs::CELL_FS_O_CREAT;

use crate::host::Lv2Host;

use crate::host::fs::flags::FS_TTY_SINK_PATHS;

#[test]
fn cell_fs_o_creat_pinned_to_octal_100() {
    // Regression: the constant was historically `0x4`, a nibble-
    // shift error from the canonical `0o100`. Pin the actual bit
    // value so a future copy-paste from a hex source surfaces
    // immediately.
    assert_eq!(CELL_FS_O_CREAT, 0o100);
    assert_eq!(CELL_FS_O_CREAT, 0x40);
}

#[test]
fn tty_sink_paths_are_pre_registered() {
    // Cross-module contract: every path in FS_TTY_SINK_PATHS
    // must also be a synthetic blob in Lv2Host::new(). Otherwise
    // a path the validator exempts from EROFS would fall through
    // to the mount table or ENOENT, and TTY-redirected writes
    // would silently vanish.
    let host = Lv2Host::new();
    for path in FS_TTY_SINK_PATHS {
        assert!(
            host.fs_store().has_path(path),
            "TTY-sink path {path:?} is in FS_TTY_SINK_PATHS but not \
             pre-registered in Lv2Host::new(); open will fall \
             through to ENOENT and TTY-redirected writes will not \
             fire",
        );
    }
}
