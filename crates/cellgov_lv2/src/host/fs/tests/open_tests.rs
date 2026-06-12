//! `sys_fs_open` dispatch tests: blob and mount path resolution, fd allocation, and the EFAULT/ENOENT/EROFS rejection paths.

use cellgov_effects::Effect;
use cellgov_ps3_abi::cell_errors;
use cellgov_ps3_abi::sys_fs::{
    CELL_FS_O_CREAT, CELL_FS_O_RDONLY, CELL_FS_O_TRUNC, CELL_FS_O_WRONLY, LV2_FS_OBJECT_ID_BASE,
};
use cellgov_time::GuestTicks;

use crate::host::Lv2Host;

use crate::host::fs::common::{
    assert_immediate, extract_fd, fs_open, run, PathRuntime, TempMountDir,
};

#[test]
fn unknown_path_returns_enoent_with_no_effects() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/dev_hdd0/game/foo/USRDIR/bar.dat\0");

    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0o1101, 0o666)),
        cell_errors::CELL_ENOENT.code,
        0,
    );
}

#[test]
fn fd_out_ptr_unmapped_returns_efault_no_effects() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/foo\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0xFFFF_FF00, 0, 0)),
        cell_errors::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn fd_out_ptr_misaligned_returns_efault() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/foo\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20001, 0, 0)),
        cell_errors::CELL_EFAULT.code,
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
        cell_errors::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn fd_out_ptr_null_returns_efault() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/foo\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0, 0, 0)),
        cell_errors::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn fs_open_bad_fd_out_ptr_takes_precedence_over_bad_path() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000);
    assert_immediate(
        run(&mut host, &rt, fs_open(0xFFFF_0000, 0xFFFF_FF00, 0, 0)),
        cell_errors::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn o_creat_for_missing_path_returns_enoent_no_effects() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/save\0");
    assert_immediate(
        run(
            &mut host,
            &rt,
            fs_open(0x10000, 0x20000, CELL_FS_O_CREAT, 0o666),
        ),
        cell_errors::CELL_ENOENT.code,
        0,
    );
}

#[test]
fn source_time_matches_runtime_tick() {
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
        fd >= LV2_FS_OBJECT_ID_BASE,
        "fs-layer fd must be in the FsStore range (>= LV2_FS_OBJECT_ID_BASE), got {fd:#x}",
    );
    assert_eq!(host.fs_store().open_fd_count(), 1);
}

#[test]
fn unknown_path_still_enoents_when_other_paths_are_registered() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/registered".into(), b"x".to_vec())
        .unwrap();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/missing\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        cell_errors::CELL_ENOENT.code,
        0,
    );
    // Invariant: failed lookup must not burn an fd id.
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
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/PARAM.SFO\0");
    let fd = extract_fd(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        0x20000,
    );
    assert!(fd >= LV2_FS_OBJECT_ID_BASE);
    assert_eq!(host.fs_store().open_fd_count(), 1);
}

#[test]
fn synthetic_output_txt_blob_pre_registered() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/output.txt\0");
    let fd = extract_fd(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        0x20000,
    );
    assert!(fd >= LV2_FS_OBJECT_ID_BASE);
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

    let baseline_blobs = host.fs_store().blob_count();
    assert!(!host.fs_store().has_path("/app_home/Data/level.xml"));

    let effects = assert_immediate(run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)), 0, 1);
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x20000);
            assert_eq!(bytes.bytes().len(), 4);
            let fd = u32::from_be_bytes(bytes.bytes().try_into().unwrap());
            assert!(fd >= LV2_FS_OBJECT_ID_BASE);
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }

    // Invariant: second open re-uses the cache; no second blob.
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
    let dir = TempMountDir::new("open_missing");
    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(crate::fs_store::FsMount::new("/app_home", dir.path.clone()).expect("valid mount"))
        .expect("registration");
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/Data/missing.bin\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        cell_errors::CELL_ENOENT.code,
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
        cell_errors::CELL_EACCES.code,
        0,
    );
}

#[test]
fn fs_open_mounted_directory_returns_enoent_in_slice3() {
    let dir = TempMountDir::new("open_dir");
    std::fs::create_dir_all(dir.path.join("savedir")).expect("subdir");
    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(crate::fs_store::FsMount::new("/app_home", dir.path.clone()).expect("valid mount"))
        .expect("registration");
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/savedir\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        cell_errors::CELL_ENOENT.code,
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
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/scratch.bin\0");
    assert_immediate(
        run(
            &mut host,
            &rt,
            fs_open(0x10000, 0x20000, CELL_FS_O_CREAT, 0o666),
        ),
        cell_errors::CELL_EROFS.code,
        0,
    );
    std::fs::remove_file(dir.path.join("scratch.bin")).expect("remove");
    assert_immediate(run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)), 0, 1);
}

#[test]
fn fs_open_with_o_wronly_on_existing_blob_returns_erofs() {
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
        cell_errors::CELL_EROFS.code,
        0,
    );
}

#[test]
fn fs_open_output_txt_with_write_flags_succeeds() {
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
