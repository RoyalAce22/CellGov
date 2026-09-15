//! Multi-root mount tests: file lookup order, shadowing, and merged
//! directory listing.

use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::fs::{CELL_FS_TYPE_DIRECTORY, CELL_FS_TYPE_REGULAR};

use crate::fs_store::FsMount;
use crate::host::Lv2Host;

use crate::host::fs::common::{
    assert_immediate, extract_fd, extract_read, extract_readdir, fs_open, fs_opendir, fs_read,
    fs_readdir, host_mounts, parse_dirent, run, PathRuntime, TempMountDir,
};

/// `/app_home` served from `update`, then `base`.
fn overlay_host(update: &TempMountDir, base: &TempMountDir) -> Lv2Host {
    let mut host = Lv2Host::new();
    host_mounts(&mut host)
        .add(
            FsMount::with_roots("/app_home", vec![update.path.clone(), base.path.clone()])
                .expect("valid mount"),
        )
        .expect("registration");
    host
}

fn read_file(host: &mut Lv2Host, path: &[u8]) -> Vec<u8> {
    let mut bytes = path.to_vec();
    bytes.push(0);
    let rt = PathRuntime::empty(0x40000).write(0x10000, &bytes);
    let fd = extract_fd(run(host, &rt, fs_open(0x10000, 0x20000, 0, 0)), 0x20000);
    let rt = PathRuntime::empty(0x40000);
    let (nread, buf) = extract_read(
        run(host, &rt, fs_read(fd, 0x20000, 64, 0x21000)),
        0x20000,
        0x21000,
    );
    let buf = buf.unwrap_or_default();
    assert_eq!(nread as usize, buf.len());
    buf
}

/// Walk a directory fd to EOF and collect each (name, dirent type).
fn list_dir(host: &mut Lv2Host, path: &[u8]) -> Vec<(String, u8)> {
    let mut bytes = path.to_vec();
    bytes.push(0);
    let rt = PathRuntime::empty(0x40000).write(0x10000, &bytes);
    let fd = extract_fd(run(host, &rt, fs_opendir(0x10000, 0x20000)), 0x20000);

    let mut entries = Vec::new();
    loop {
        let rt = PathRuntime::empty(0x40000);
        let (blob, nread) = extract_readdir(
            run(host, &rt, fs_readdir(fd, 0x20000, 0x21000)),
            0x20000,
            0x21000,
        );
        if nread == 0 {
            break;
        }
        let (d_type, _, name) = parse_dirent(&blob);
        entries.push((name, d_type));
    }
    entries
}

fn names(entries: &[(String, u8)]) -> Vec<&str> {
    entries.iter().map(|(n, _)| n.as_str()).collect()
}

#[test]
fn earlier_root_shadows_the_same_path_in_a_later_root() {
    let update = TempMountDir::new("overlay_shadow_update");
    let base = TempMountDir::new("overlay_shadow_base");
    update.write("Data/level.xml", b"update");
    base.write("Data/level.xml", b"base");
    let mut host = overlay_host(&update, &base);

    assert_eq!(
        read_file(&mut host, b"/app_home/Data/level.xml"),
        b"update".to_vec()
    );
}

#[test]
fn a_path_missing_from_the_earlier_root_falls_through() {
    let update = TempMountDir::new("overlay_fallthrough_update");
    let base = TempMountDir::new("overlay_fallthrough_base");
    update.write("Data/patched.xml", b"update");
    base.write("Data/original.xml", b"base");
    let mut host = overlay_host(&update, &base);

    assert_eq!(
        read_file(&mut host, b"/app_home/Data/original.xml"),
        b"base".to_vec()
    );
}

#[test]
fn a_path_in_no_root_returns_enoent() {
    let update = TempMountDir::new("overlay_miss_update");
    let base = TempMountDir::new("overlay_miss_base");
    let mut host = overlay_host(&update, &base);

    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/Data/absent.xml\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        errno::CELL_ENOENT.code,
        0,
    );
}

#[test]
fn two_lookups_of_the_same_path_read_identical_bytes() {
    let update = TempMountDir::new("overlay_repeat_update");
    let base = TempMountDir::new("overlay_repeat_base");
    update.write("Data/level.xml", b"update");
    base.write("Data/level.xml", b"base");
    let mut host = overlay_host(&update, &base);

    let first = read_file(&mut host, b"/app_home/Data/level.xml");
    let second = read_file(&mut host, b"/app_home/Data/level.xml");
    assert_eq!(first, b"update".to_vec());
    assert_eq!(first, second);
}

#[test]
fn a_root_whose_path_runs_through_a_file_is_treated_as_absent() {
    let update = TempMountDir::new("overlay_notdir_update");
    let base = TempMountDir::new("overlay_notdir_base");
    // `Data` is a regular file under `update`, so nothing can live
    // below it there. Hosts report that as NotFound or NotADirectory
    // by family; both must read as "not under this root".
    update.write("Data", b"update");
    base.write("Data/original.xml", b"base");
    let mut host = overlay_host(&update, &base);

    assert_eq!(
        read_file(&mut host, b"/app_home/Data/original.xml"),
        b"base".to_vec()
    );
}

#[test]
fn a_root_that_does_not_exist_at_all_is_skipped() {
    let update = TempMountDir::new("overlay_absent_root_update");
    let base = TempMountDir::new("overlay_absent_root_base");
    base.write("Data/original.xml", b"base");
    let mut host = Lv2Host::new();
    host_mounts(&mut host)
        .add(
            FsMount::with_roots(
                "/app_home",
                vec![update.path.join("never_created"), base.path.clone()],
            )
            .expect("valid mount"),
        )
        .expect("registration");

    assert_eq!(
        read_file(&mut host, b"/app_home/Data/original.xml"),
        b"base".to_vec()
    );
}

#[test]
fn the_third_root_answers_when_the_first_two_miss() {
    let first = TempMountDir::new("overlay_three_first");
    let second = TempMountDir::new("overlay_three_second");
    let third = TempMountDir::new("overlay_three_third");
    first.write("Data/a.xml", b"first");
    second.write("Data/b.xml", b"second");
    third.write("Data/c.xml", b"third");
    let mut host = Lv2Host::new();
    host_mounts(&mut host)
        .add(
            FsMount::with_roots(
                "/app_home",
                vec![first.path.clone(), second.path.clone(), third.path.clone()],
            )
            .expect("valid mount"),
        )
        .expect("registration");

    assert_eq!(
        read_file(&mut host, b"/app_home/Data/c.xml"),
        b"third".to_vec()
    );
    let entries = list_dir(&mut host, b"/app_home/Data");
    assert_eq!(names(&entries), ["a.xml", "b.xml", "c.xml"]);
}

#[test]
fn listing_orders_names_by_byte_value_not_case_folded() {
    let update = TempMountDir::new("overlay_byteorder_update");
    let base = TempMountDir::new("overlay_byteorder_base");
    // 'Z' is 0x5a, '_' is 0x5f, 'a' is 0x61: byte order and any
    // case-folding order disagree about all three.
    update.write("Data/Z.xml", b"u");
    update.write("Data/a.xml", b"u");
    base.write("Data/_.xml", b"b");
    let mut host = overlay_host(&update, &base);

    let entries = list_dir(&mut host, b"/app_home/Data");
    assert_eq!(names(&entries), ["Z.xml", "_.xml", "a.xml"]);
}

#[test]
fn listing_merges_both_roots_in_sorted_order() {
    let update = TempMountDir::new("overlay_list_update");
    let base = TempMountDir::new("overlay_list_base");
    update.write("Data/zzz.xml", b"u");
    update.write("Data/b.xml", b"u");
    base.write("Data/a.xml", b"b");
    base.write("Data/m.xml", b"b");
    let mut host = overlay_host(&update, &base);

    let entries = list_dir(&mut host, b"/app_home/Data");
    assert_eq!(names(&entries), ["a.xml", "b.xml", "m.xml", "zzz.xml"]);
}

#[test]
fn listing_reports_a_name_held_by_both_roots_once() {
    let update = TempMountDir::new("overlay_dedup_update");
    let base = TempMountDir::new("overlay_dedup_base");
    update.write("Data/shared.xml", b"update");
    update.write("Data/only_update.xml", b"update");
    base.write("Data/shared.xml", b"base");
    base.write("Data/only_base.xml", b"base");
    let mut host = overlay_host(&update, &base);

    let entries = list_dir(&mut host, b"/app_home/Data");
    assert_eq!(
        names(&entries),
        ["only_base.xml", "only_update.xml", "shared.xml"]
    );
}

#[test]
fn a_deduplicated_name_keeps_the_earlier_root_type() {
    let update = TempMountDir::new("overlay_type_update");
    let base = TempMountDir::new("overlay_type_base");
    update.mkdir("Data/entry");
    base.write("Data/entry", b"base");
    let mut host = overlay_host(&update, &base);

    let entries = list_dir(&mut host, b"/app_home/Data");
    assert_eq!(entries, vec![("entry".to_string(), CELL_FS_TYPE_DIRECTORY)]);
}

#[test]
fn a_directory_only_in_the_later_root_still_lists() {
    let update = TempMountDir::new("overlay_dironly_update");
    let base = TempMountDir::new("overlay_dironly_base");
    update.mkdir("Data");
    base.write("Data/original.xml", b"base");
    let mut host = overlay_host(&update, &base);

    let entries = list_dir(&mut host, b"/app_home/Data");
    assert_eq!(
        entries,
        vec![("original.xml".to_string(), CELL_FS_TYPE_REGULAR)]
    );
}

#[test]
fn a_file_shadowing_a_directory_makes_opendir_enotdir() {
    let update = TempMountDir::new("overlay_enotdir_update");
    let base = TempMountDir::new("overlay_enotdir_base");
    update.write("Data", b"update");
    base.write("Data/original.xml", b"base");
    let mut host = overlay_host(&update, &base);

    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/Data\0");
    assert_immediate(
        run(&mut host, &rt, fs_opendir(0x10000, 0x20000)),
        errno::CELL_ENOTDIR.code,
        0,
    );
}

#[test]
fn a_directory_shadowing_a_file_makes_open_enoent() {
    let update = TempMountDir::new("overlay_enoent_update");
    let base = TempMountDir::new("overlay_enoent_base");
    update.mkdir("Data/entry");
    base.write("Data/entry", b"base");
    let mut host = overlay_host(&update, &base);

    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/Data/entry\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        errno::CELL_ENOENT.code,
        0,
    );
}

#[test]
fn a_one_root_mount_reads_and_lists_exactly_its_root() {
    let only = TempMountDir::new("overlay_single_root");
    only.write("Data/a.xml", b"a");
    only.write("Data/b.xml", b"b");
    only.mkdir("Data/sub");
    let mut host = Lv2Host::new();
    host_mounts(&mut host)
        .add(FsMount::new("/app_home", only.path.clone()).expect("valid mount"))
        .expect("registration");

    assert_eq!(read_file(&mut host, b"/app_home/Data/a.xml"), b"a".to_vec());
    let entries = list_dir(&mut host, b"/app_home/Data");
    assert_eq!(
        entries,
        vec![
            ("a.xml".to_string(), CELL_FS_TYPE_REGULAR),
            ("b.xml".to_string(), CELL_FS_TYPE_REGULAR),
            ("sub".to_string(), CELL_FS_TYPE_DIRECTORY),
        ]
    );
}
