use cellgov_ps3_abi::sys_fs::CELL_FS_O_CREAT;

use crate::host::Lv2Host;

use super::super::flags::FS_TTY_SINK_PATHS;

#[test]
fn cell_fs_o_creat_pinned_to_octal_100() {
    // Regression: the constant was historically `0x4` which is a
    // nibble-shift error from the PSL1GHT canonical `0o100`.
    // Pin the actual bit value so a future copy-paste from a
    // hex source surfaces immediately.
    assert_eq!(CELL_FS_O_CREAT, 0o100);
    assert_eq!(CELL_FS_O_CREAT, 0x40);
}

#[test]
fn tty_sink_paths_are_pre_registered() {
    // Cross-module contract: every path in FS_TTY_SINK_PATHS
    // must also be a synthetic blob in Lv2Host::new(). If this
    // ever fails, the validator exempted a path from EROFS that
    // the FsStore knows nothing about -- the open would then
    // fall through to the mount table or ENOENT, the title's
    // fopen would silently fail, and TTY output would vanish
    // (this is exactly how the cpu_ppu_branch ps3autotest
    // regressed during slice 3 development).
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
