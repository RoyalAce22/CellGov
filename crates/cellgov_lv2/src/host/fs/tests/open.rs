//! `dispatch_fs_open` tests: error precedence on out-pointer / path
//! validation, manifest-blob routing, mount-resolve + cache, flag
//! validation, and TTY-sink exemption.

use cellgov_effects::Effect;
use cellgov_ps3_abi::cell_errors as errno;
use cellgov_ps3_abi::sys_fs::{
    CELL_FS_O_CREAT, CELL_FS_O_RDONLY, CELL_FS_O_TRUNC, CELL_FS_O_WRONLY,
};
use cellgov_time::GuestTicks;

use crate::fs_store::FD_BASE;
use crate::host::Lv2Host;

use super::common::{assert_immediate, extract_fd, fs_open, run, PathRuntime, TempMountDir};

#[test]
fn unknown_path_returns_enoent_with_no_effects() {
    // POSIX: fd_out_ptr is undefined on error; real PS3 does
    // not write it on ENOENT. The flags here include the
    // canonical O_TRUNC | O_CREAT | O_WRONLY combo (`0o1101`)
    // a title might use to create-or-truncate -- existence
    // wins, so ENOENT regardless of write flags.
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/dev_hdd0/game/foo/USRDIR/bar.dat\0");

    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0o1101, 0o666)),
        errno::CELL_ENOENT.code,
        0,
    );
}

#[test]
fn fd_out_ptr_unmapped_returns_efault_no_effects() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/foo\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0xFFFF_FF00, 0, 0)),
        errno::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn fd_out_ptr_misaligned_returns_efault() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/foo\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20001, 0, 0)),
        errno::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn fd_out_ptr_in_reserved_region_returns_efault() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000)
        .write(0x10000, b"/foo\0")
        .reserve(0x30000, 0x31000);
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x30100, 0, 0)),
        errno::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn fd_out_ptr_null_returns_efault() {
    // NULL passes the alignment check (`0 & 3 == 0`) so the
    // out_ptr_writable helper carries the explicit non-zero
    // guard. Without it a buggy guest passing fd_out_ptr=0 would
    // skate through.
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/foo\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0, 0, 0)),
        errno::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn fs_open_bad_fd_out_ptr_takes_precedence_over_bad_path() {
    // Both `fd_out_ptr` and `path_ptr` unmapped: the dispatch
    // checks fd_out_ptr first (cheaper -- no path scan
    // required). Pin the order so a future swap is visible.
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000);
    assert_immediate(
        run(&mut host, &rt, fs_open(0xFFFF_0000, 0xFFFF_FF00, 0, 0)),
        errno::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn o_creat_for_missing_path_returns_enoent_no_effects() {
    // Existence wins over flag-error: a missing path with
    // O_CREAT still ENOENTs (the kernel checks existence before
    // it checks write semantics). The dispatch returns no
    // effects -- POSIX says fd_out_ptr is undefined on error
    // and real PS3 does not write it, so writing a synthetic
    // zero would diverge in cross-runner compare.
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/save\0");
    assert_immediate(
        run(
            &mut host,
            &rt,
            fs_open(0x10000, 0x20000, CELL_FS_O_CREAT, 0o666),
        ),
        errno::CELL_ENOENT.code,
        0,
    );
}

#[test]
fn source_time_matches_runtime_tick() {
    // Pin source_time on the successful-open effect (the
    // ENOENT path emits no effects). Register a manifest blob
    // so the open hits the FsStore and produces the fd-write.
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"x".to_vec())
        .unwrap();
    let rt = PathRuntime::empty(0x40000)
        .write(0x10000, b"/foo\0")
        .with_tick(GuestTicks::new(42));
    let effects = assert_immediate(run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)), 0, 1);
    match &effects[0] {
        Effect::SharedWriteIntent { source_time, .. } => {
            assert_eq!(source_time.raw(), 42);
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn registered_blob_path_routes_through_fs_layer() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/registered/foo".into(), b"bytes".to_vec())
        .unwrap();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/registered/foo\0");
    let fd = extract_fd(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        0x20000,
    );
    assert!(
        fd >= FD_BASE,
        "fs-layer fd must be in the FsStore range (>= FD_BASE), got {fd:#x}",
    );
    // FsStore tracked the open; kernel-side fs_fd_count tracked
    // it too -- both routes through sys_process_get_number_of_object
    // must agree on a single live fd.
    assert_eq!(host.fs_store().open_fd_count(), 1);
}

#[test]
fn unknown_path_still_enoents_when_other_paths_are_registered() {
    // Mixed-state regression: a manifest with `/registered`
    // present must not change ENOENT for a different,
    // unregistered path.
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/registered".into(), b"x".to_vec())
        .unwrap();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/missing\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        errno::CELL_ENOENT.code,
        0,
    );
    // Failed FS lookup must not burn a host-side fd id.
    assert_eq!(host.fs_store().open_fd_count(), 0);
}

#[test]
fn two_opens_of_same_registered_path_return_distinct_fds() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/dup".into(), b"x".to_vec())
        .unwrap();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/dup\0");
    let fd_a = extract_fd(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        0x20000,
    );
    let fd_b = extract_fd(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        0x20000,
    );
    assert_ne!(fd_a, fd_b, "second open must allocate a fresh fd");
    assert_eq!(host.fs_store().open_fd_count(), 2);
}

#[test]
fn synthetic_param_sfo_blob_pre_registered() {
    // PSL1GHT-test ELFs probe `/app_home/PARAM.SFO` for
    // existence before a real boot. The host registers it as
    // a zero-byte blob in `Lv2Host::new()` so the open routes
    // through FsStore (single allocator -- no fd-aliasing risk
    // with a separate whitelist allocator).
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/PARAM.SFO\0");
    let fd = extract_fd(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        0x20000,
    );
    assert!(fd >= FD_BASE);
    assert_eq!(host.fs_store().open_fd_count(), 1);
}

#[test]
fn synthetic_output_txt_blob_pre_registered() {
    // PSL1GHT tests fopen + fwrite + fclose to output.txt. The
    // sister of PARAM.SFO -- both registered up front so the
    // probe-for-existence open succeeds with a real FsStore fd.
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/output.txt\0");
    let fd = extract_fd(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        0x20000,
    );
    assert!(fd >= FD_BASE);
    assert_eq!(host.fs_store().open_fd_count(), 1);
}

#[test]
fn fs_open_resolves_via_mount_and_caches_blob() {
    let dir = TempMountDir::new("open_resolves");
    dir.write("Data/level.xml", b"<level/>");
    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(crate::fs_store::FsMount::new("/app_home", dir.path.clone()).expect("valid mount"))
        .expect("registration");
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/Data/level.xml\0");

    // Pre-state: the path is not in the FsStore.
    let baseline_blobs = host.fs_store().blob_count();
    assert!(!host.fs_store().has_path("/app_home/Data/level.xml"));

    let effects = assert_immediate(run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)), 0, 1);
    // Effect carries the new fd written to fd_out_ptr.
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x20000);
            assert_eq!(bytes.bytes().len(), 4);
            let fd = u32::from_be_bytes(bytes.bytes().try_into().unwrap());
            assert!(fd >= FD_BASE);
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }

    // Post-state: the resolution registered the blob in the
    // FsStore. Second open re-uses the cache without consulting
    // the host filesystem at all (asserted via FsStore state
    // rather than by deleting the host file -- avoids a host-fs
    // side channel that could behave differently on Windows
    // sharing-violation rules or future CI ports).
    assert_eq!(host.fs_store().blob_count(), baseline_blobs + 1);
    assert!(host.fs_store().has_path("/app_home/Data/level.xml"));
    let effects2 = assert_immediate(run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)), 0, 1);
    assert_eq!(effects2.len(), 1);
    assert_eq!(
        host.fs_store().blob_count(),
        baseline_blobs + 1,
        "second open must not register a second blob"
    );
}

#[test]
fn fs_open_mounted_missing_returns_enoent() {
    // A path under a mount whose host file does not exist must
    // ENOENT (not silently fall back to anywhere else).
    let dir = TempMountDir::new("open_missing");
    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(crate::fs_store::FsMount::new("/app_home", dir.path.clone()).expect("valid mount"))
        .expect("registration");
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/Data/missing.bin\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        errno::CELL_ENOENT.code,
        0,
    );
}

#[test]
fn fs_open_mount_path_traversal_returns_eacces() {
    let dir = TempMountDir::new("open_traversal");
    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(crate::fs_store::FsMount::new("/app_home", dir.path.clone()).expect("valid mount"))
        .expect("registration");
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/../etc/passwd\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        errno::CELL_EACCES.code,
        0,
    );
}

#[test]
fn fs_open_mounted_directory_returns_enoent_in_slice3() {
    // Slice 3 only handles regular files; opendir/readdir
    // arrive in slice 4. A guest open targeting a directory
    // surfaces ENOENT until then so titles do not see a
    // synthetic fd that has no readable bytes behind it.
    let dir = TempMountDir::new("open_dir");
    std::fs::create_dir_all(dir.path.join("savedir")).expect("subdir");
    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(crate::fs_store::FsMount::new("/app_home", dir.path.clone()).expect("valid mount"))
        .expect("registration");
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/savedir\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        errno::CELL_ENOENT.code,
        0,
    );
}

#[test]
fn fs_open_o_creat_under_mount_returns_erofs() {
    let dir = TempMountDir::new("open_creat");
    dir.write("scratch.bin", b"existing");
    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(crate::fs_store::FsMount::new("/app_home", dir.path.clone()).expect("valid mount"))
        .expect("registration");
    // O_CREAT under a read-only mount: the cache happens
    // first (so the path is now in the FsStore), then the
    // EROFS check fires before fd allocation. Repeat opens
    // without O_CREAT must succeed, which is exactly the
    // single-write determinism contract.
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/scratch.bin\0");
    assert_immediate(
        run(
            &mut host,
            &rt,
            fs_open(0x10000, 0x20000, CELL_FS_O_CREAT, 0o666),
        ),
        errno::CELL_EROFS.code,
        0,
    );
    // Subsequent read-only open hits the cache (no disk).
    std::fs::remove_file(dir.path.join("scratch.bin")).expect("remove");
    assert_immediate(run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)), 0, 1);
}

#[test]
fn fs_open_with_o_wronly_on_existing_blob_returns_erofs() {
    // Companion to `fs_open_o_creat_under_mount_returns_erofs`
    // for the manifest path. A registered blob with a write
    // flag must EROFS, not silently succeed with a read-only
    // fd.
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"x".to_vec())
        .unwrap();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/foo\0");
    assert_immediate(
        run(
            &mut host,
            &rt,
            fs_open(0x10000, 0x20000, CELL_FS_O_WRONLY, 0o666),
        ),
        errno::CELL_EROFS.code,
        0,
    );
}

#[test]
fn fs_open_output_txt_with_write_flags_succeeds() {
    // PSL1GHT-test fixture: fopen("/app_home/output.txt", "w")
    // decodes to O_WRONLY | O_CREAT | O_TRUNC. The synthetic
    // blob is in FsStore so existence is satisfied; the
    // tty-sink whitelist exempts it from the EROFS branch so
    // the open succeeds and fs_write redirects bytes to
    // tty_log.
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/output.txt\0");
    let flags = CELL_FS_O_WRONLY | CELL_FS_O_CREAT | CELL_FS_O_TRUNC;
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, flags, 0o666)),
        0,
        1,
    );
}

#[test]
fn fs_open_with_o_rdonly_on_existing_blob_succeeds() {
    // Negative for the EROFS test above: explicit RDONLY
    // (which is `0`) must still open. Pinning this means an
    // overzealous flag rejection is visible.
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"x".to_vec())
        .unwrap();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/foo\0");
    assert_immediate(
        run(
            &mut host,
            &rt,
            fs_open(0x10000, 0x20000, CELL_FS_O_RDONLY, 0),
        ),
        0,
        1,
    );
}
