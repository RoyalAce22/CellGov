//! The file, content, PRX and shared-memory state keeps its partial of
//! the sync-state sum through every path that changes it.

use super::*;
use crate::fs_store::{DirEntry, FsStore, SeekWhence};
use crate::host::mmapper::MmapperHandle;
use crate::image::LsSegment;
use crate::prx_registry::LoadedPrxRegistry;

/// Apply `$change`; the host partial must move and equal the rebuild,
/// in release too.
macro_rules! step {
    ($host:ident, $h:ident, $what:expr, $change:expr) => {{
        let _ = $change;
        let now = $host.sync_partial();
        assert_eq!(now, $host.sync_partial_from_scratch(), "after {}", $what);
        assert_ne!(now, $h, "{} did not move the partial", $what);
        $h = now;
        let _ = $h;
    }};
}

#[test]
fn every_file_content_prx_and_shared_memory_change_moves_the_partial() {
    let mut host = Lv2Host::new();
    let mut h = host.sync_partial();
    step!(
        host,
        h,
        "blob",
        host.state
            .fs_store
            .register_blob("/a".into(), vec![1, 2, 3])
    );
    let fd = host.state.fs_store.open_fd("/a").unwrap();
    assert_ne!(host.sync_partial(), h, "open did not move the partial");
    h = host.sync_partial();
    step!(host, h, "read", host.state.fs_store.read_at(fd, 2));
    step!(
        host,
        h,
        "seek",
        host.state.fs_store.seek(fd, 0, SeekWhence::Set)
    );
    let dir = host
        .state
        .fs_store
        .open_dir(vec![DirEntry {
            name: "x".into(),
            is_directory: false,
        }])
        .unwrap();
    assert_ne!(host.sync_partial(), h, "open_dir did not move the partial");
    h = host.sync_partial();
    step!(host, h, "dir read", host.state.fs_store.read_dir_entry(dir));
    step!(host, h, "dir close", host.state.fs_store.close_dir(dir));
    step!(host, h, "fd close", host.state.fs_store.close_fd(fd));
    step!(
        host,
        h,
        "image",
        host.content_store_mut().register(b"/spu.elf", vec![9])
    );
    let user = host.content_store_mut().register_user_image(
        0x100,
        vec![LsSegment {
            ls_start: 0,
            bytes: vec![1],
        }],
    );
    assert_ne!(
        host.sync_partial(),
        h,
        "user image did not move the partial"
    );
    h = host.sync_partial();
    step!(
        host,
        h,
        "user image withdraw",
        host.content_store_mut().withdraw_user_image(user)
    );
    let id = host.state.prx_registry.register(
        "libfoo".into(),
        "Foo".into(),
        0x1000,
        0x2000,
        0x1800,
        None,
        None,
    );
    assert_ne!(
        host.sync_partial(),
        h,
        "prx register did not move the partial"
    );
    h = host.sync_partial();
    step!(
        host,
        h,
        "prx start",
        host.state.prx_registry.mark_started(id)
    );
    step!(
        host,
        h,
        "prx stop 1",
        host.state.prx_registry.begin_stop(id)
    );
    step!(
        host,
        h,
        "prx stop 2",
        host.state.prx_registry.finish_stop(id)
    );
    step!(
        host,
        h,
        "prx withdraw",
        host.state.prx_registry.withdraw_removable(id)
    );
    step!(
        host,
        h,
        "mmapper handle",
        host.state.mmapper_handles.insert(
            5,
            MmapperHandle {
                size: 0x10000,
                align: 0x10000,
            },
        )
    );
    step!(
        host,
        h,
        "ipc key",
        host.state.mmapper_ipc.insert(0x8006_0100_0000_0010, 5)
    );
    step!(
        host,
        h,
        "memory container",
        host.state.memory_containers.insert(7, ())
    );
}

#[test]
fn blob_partials_ignore_registration_order_and_follow_their_paths() {
    let build = |order: [(&str, u8); 2]| {
        let mut s = FsStore::new();
        for (path, byte) in order {
            s.register_blob(path.into(), vec![byte]).unwrap();
        }
        s.sync_partial()
    };
    assert_eq!(build([("/a", 1), ("/b", 2)]), build([("/b", 2), ("/a", 1)]));
    assert_ne!(build([("/a", 1), ("/b", 2)]), build([("/a", 2), ("/b", 1)]));
}

#[test]
fn an_open_then_close_of_any_blob_leaves_the_same_partial() {
    let build = |path: &str| {
        let mut s = FsStore::new();
        s.register_blob("/a".into(), vec![1]).unwrap();
        s.register_blob("/b".into(), vec![2]).unwrap();
        let fd = s.open_fd(path).unwrap();
        s.close_fd(fd).unwrap();
        s.sync_partial()
    };
    assert_eq!(build("/a"), build("/b"));
}

#[test]
fn an_open_fd_partial_follows_the_path_it_names() {
    let build = |path: &str| {
        let mut s = FsStore::new();
        s.register_blob("/a".into(), vec![1]).unwrap();
        s.register_blob("/b".into(), vec![1]).unwrap();
        s.open_fd(path).unwrap();
        s.sync_partial()
    };
    assert_ne!(build("/a"), build("/b"));
}

#[test]
fn a_dir_partial_follows_entry_order_and_kind() {
    let build = |entries: [(&str, bool); 2]| {
        let mut s = FsStore::new();
        s.open_dir(
            entries
                .iter()
                .map(|&(name, is_directory)| DirEntry {
                    name: name.into(),
                    is_directory,
                })
                .collect(),
        )
        .unwrap();
        s.sync_partial()
    };
    let base = build([("a", false), ("b", false)]);
    assert_ne!(base, build([("b", false), ("a", false)]));
    assert_ne!(base, build([("a", true), ("b", false)]));
}

#[test]
fn an_image_partial_follows_the_handle_each_path_got() {
    let build = |order: [&[u8]; 2]| {
        let mut s = crate::image::ContentStore::new();
        for path in order {
            s.register(path, vec![1]);
        }
        s.sync_partial()
    };
    let (a, b): (&[u8], &[u8]) = (b"/a", b"/b");
    assert_ne!(build([a, b]), build([b, a]));
}

#[test]
fn a_user_image_partial_follows_segment_order_and_count() {
    let build = |segments: Vec<LsSegment>| {
        let mut s = crate::image::ContentStore::new();
        s.register_user_image(0x80, segments);
        s.sync_partial()
    };
    let seg = |ls_start: u32, bytes: Vec<u8>| LsSegment { ls_start, bytes };
    let base = build(vec![seg(0x100, vec![1]), seg(0x200, vec![2])]);
    assert_ne!(base, build(vec![seg(0x200, vec![2]), seg(0x100, vec![1])]));
    assert_ne!(base, build(vec![seg(0x100, vec![2]), seg(0x200, vec![1])]));
    assert_ne!(build(vec![]), build(vec![seg(0, vec![])]));
}

/// Computed outside the crate from the SplitMix64 key streams of the
/// sync-state lanes and the mixer digest.
#[test]
fn fs_blob_partial_wire_format_golden() {
    let mut s = FsStore::new();
    s.register_blob("/a".into(), vec![1, 2, 3]).unwrap();
    assert_eq!(s.sync_partial(), 0xff55_48a6_ba64_3d98_135c_9062_9758_d770);
}

/// Computed outside the crate from the SplitMix64 key streams of the
/// sync-state lanes and the mixer digest.
#[test]
fn prx_partial_wire_format_golden() {
    let mut r = LoadedPrxRegistry::new();
    r.register(
        "libfoo".into(),
        "Foo module".into(),
        0x1000,
        0x2000,
        0x1800,
        Some(0x1100),
        None,
    );
    assert_eq!(r.sync_partial(), 0x4d4d_31cc_5493_91a8_a673_62e4_ef82_e0cb);
}
