//! FsStore tests covering blob registration, fd/dir lifecycle, seek semantics, and state-hash sensitivity.

use super::*;

fn fs_with(path: &str, bytes: &[u8]) -> FsStore {
    let mut s = FsStore::new();
    s.register_blob(path.to_owned(), bytes.to_vec()).unwrap();
    s
}

#[test]
fn lookup_returns_registered_blob() {
    let s = fs_with("/foo", b"hello");
    assert_eq!(s.lookup_blob("/foo"), Some(b"hello".as_slice()));
    assert_eq!(s.lookup_blob("/bar"), None);
}

#[test]
fn open_unknown_path_returns_unknown_path() {
    let mut s = FsStore::new();
    assert_eq!(s.open_fd("/missing"), Err(FsError::UnknownPath));
}

#[test]
fn fd_allocator_is_monotonic_and_distinct_per_open() {
    let mut s = fs_with("/foo", b"x");
    let a = s.open_fd("/foo").unwrap();
    let b = s.open_fd("/foo").unwrap();
    assert_eq!(a, LV2_FS_OBJECT_ID_BASE);
    assert_eq!(b, LV2_FS_OBJECT_ID_BASE + 1);
    assert_ne!(a, b);
}

#[test]
fn read_advances_offset_and_returns_next_slice() {
    let mut s = fs_with("/foo", b"abcdef");
    let fd = s.open_fd("/foo").unwrap();
    assert_eq!(s.read_at(fd, 3).unwrap(), b"abc");
    assert_eq!(s.read_at(fd, 3).unwrap(), b"def");
}

#[test]
fn read_at_eof_returns_empty() {
    let mut s = fs_with("/foo", b"abc");
    let fd = s.open_fd("/foo").unwrap();
    let _ = s.read_at(fd, 3).unwrap();
    assert!(s.read_at(fd, 3).unwrap().is_empty());
}

#[test]
fn read_clamps_to_remaining_bytes() {
    let mut s = fs_with("/foo", b"abc");
    let fd = s.open_fd("/foo").unwrap();
    assert_eq!(s.read_at(fd, 100).unwrap(), b"abc");
}

#[test]
fn close_releases_fd_and_subsequent_ops_fail() {
    let mut s = fs_with("/foo", b"x");
    let fd = s.open_fd("/foo").unwrap();
    assert!(s.close_fd(fd).is_ok());
    assert_eq!(s.close_fd(fd), Err(FsError::UnknownFd));
    assert_eq!(s.read_at(fd, 1), Err(FsError::UnknownFd));
    assert_eq!(s.fstat(fd), Err(FsError::UnknownFd));
}

#[test]
fn seek_set_jumps_to_absolute_offset() {
    let mut s = fs_with("/foo", b"abcdef");
    let fd = s.open_fd("/foo").unwrap();
    assert_eq!(s.seek(fd, 4, SeekWhence::Set).unwrap(), 4);
    assert_eq!(s.read_at(fd, 10).unwrap(), b"ef");
}

#[test]
fn seek_cur_advances_relative() {
    let mut s = fs_with("/foo", b"abcdef");
    let fd = s.open_fd("/foo").unwrap();
    let _ = s.read_at(fd, 2).unwrap();
    assert_eq!(s.seek(fd, 2, SeekWhence::Cur).unwrap(), 4);
    assert_eq!(s.read_at(fd, 10).unwrap(), b"ef");
}

#[test]
fn seek_end_returns_size() {
    let mut s = fs_with("/foo", b"abcdef");
    let fd = s.open_fd("/foo").unwrap();
    assert_eq!(s.seek(fd, 0, SeekWhence::End).unwrap(), 6);
}

#[test]
fn seek_negative_past_zero_returns_out_of_range() {
    let mut s = fs_with("/foo", b"abcdef");
    let fd = s.open_fd("/foo").unwrap();
    assert_eq!(
        s.seek(fd, -1, SeekWhence::Set),
        Err(FsError::SeekOutOfRange)
    );
}

#[test]
fn seek_past_end_is_allowed_and_subsequent_read_returns_empty() {
    let mut s = fs_with("/foo", b"abc");
    let fd = s.open_fd("/foo").unwrap();
    assert_eq!(s.seek(fd, 100, SeekWhence::Set).unwrap(), 100);
    assert!(s.read_at(fd, 10).unwrap().is_empty());
}

#[test]
fn stat_path_returns_size() {
    let s = fs_with("/foo", b"hello world");
    assert_eq!(s.stat_path("/foo").unwrap().size, 11);
}

#[test]
fn stat_unknown_path_returns_unknown_path() {
    let s = FsStore::new();
    assert_eq!(s.stat_path("/foo"), Err(FsError::UnknownPath));
}

#[test]
fn fstat_returns_size_for_open_fd() {
    let mut s = fs_with("/foo", b"hello");
    let fd = s.open_fd("/foo").unwrap();
    assert_eq!(s.fstat(fd).unwrap().size, 5);
}

#[test]
fn whence_decoder_rejects_out_of_range() {
    assert_eq!(SeekWhence::from_guest(0), Some(SeekWhence::Set));
    assert_eq!(SeekWhence::from_guest(1), Some(SeekWhence::Cur));
    assert_eq!(SeekWhence::from_guest(2), Some(SeekWhence::End));
    assert_eq!(SeekWhence::from_guest(3), None);
    assert_eq!(SeekWhence::from_guest(0xFFFF_FFFF), None);
}

#[test]
fn state_hash_changes_on_blob_registration() {
    let s0 = FsStore::new();
    let mut s1 = FsStore::new();
    s1.register_blob("/foo".into(), b"x".to_vec()).unwrap();
    assert_ne!(s0.state_hash(), s1.state_hash());
}

#[test]
fn state_hash_changes_on_content_swap() {
    let mut s1 = FsStore::new();
    s1.register_blob("/foo".into(), b"x".to_vec()).unwrap();
    let h1 = s1.state_hash();
    let mut s2 = FsStore::new();
    s2.register_blob("/foo".into(), b"y".to_vec()).unwrap();
    let h2 = s2.state_hash();
    assert_ne!(h1, h2);
}

#[test]
fn state_hash_is_insertion_order_independent() {
    let paths = ["/z", "/a", "/m", "/b", "/y", "/c"];
    let mut a = FsStore::new();
    for p in paths {
        a.register_blob(p.into(), p.as_bytes().to_vec()).unwrap();
    }
    let mut b = FsStore::new();
    for p in paths.iter().rev() {
        b.register_blob((*p).into(), p.as_bytes().to_vec()).unwrap();
    }
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn duplicate_register_returns_path_already_registered() {
    let mut s = FsStore::new();
    s.register_blob("/foo".into(), b"first".to_vec()).unwrap();
    assert_eq!(
        s.register_blob("/foo".into(), b"second".to_vec()),
        Err(FsError::PathAlreadyRegistered),
    );
    assert_eq!(s.lookup_blob("/foo"), Some(b"first".as_slice()));
}

#[test]
fn fd_exhaustion_at_u32_max_is_explicit() {
    let mut s = fs_with("/foo", b"x");
    s.next_fd = u32::MAX - 1;
    let last = s.open_fd("/foo").unwrap();
    assert_eq!(last, u32::MAX - 1);
    assert_eq!(s.open_fd("/foo"), Err(FsError::FdExhausted));
    assert_eq!(s.open_fd_count(), 1);
}

#[test]
fn unknown_path_open_does_not_burn_an_fd() {
    let mut s = fs_with("/foo", b"x");
    let h0 = s.state_hash();
    assert_eq!(s.open_fd("/missing"), Err(FsError::UnknownPath));
    assert_eq!(s.state_hash(), h0);
}

#[test]
fn seek_positive_overflow_returns_out_of_range() {
    let mut s = fs_with("/foo", b"x");
    let fd = s.open_fd("/foo").unwrap();
    assert_eq!(
        s.seek(fd, i64::MAX, SeekWhence::Set).unwrap(),
        i64::MAX as u64
    );
    assert_eq!(s.seek(fd, i64::MAX, SeekWhence::Cur).unwrap(), u64::MAX - 1);
    assert_eq!(s.seek(fd, 1, SeekWhence::Cur).unwrap(), u64::MAX);
    assert_eq!(s.seek(fd, 1, SeekWhence::Cur), Err(FsError::SeekOutOfRange));
}

#[test]
fn zero_byte_read_does_not_disturb_offset() {
    let mut s = fs_with("/foo", b"abc");
    let fd = s.open_fd("/foo").unwrap();
    s.seek(fd, 100, SeekWhence::Set).unwrap();
    let _ = s.read_at(fd, 0).unwrap();
    assert_eq!(s.seek(fd, 0, SeekWhence::Cur).unwrap(), 100);
}

#[test]
fn huge_max_bytes_does_not_overflow_usize() {
    let mut s = fs_with("/foo", b"abc");
    let fd = s.open_fd("/foo").unwrap();
    assert_eq!(s.read_at(fd, usize::MAX).unwrap(), b"abc");
}

#[test]
fn empty_blob_open_read_stat() {
    let mut s = fs_with("/empty", b"");
    let fd = s.open_fd("/empty").unwrap();
    assert!(s.read_at(fd, 100).unwrap().is_empty());
    assert_eq!(s.fstat(fd).unwrap().size, 0);
    assert_eq!(s.stat_path("/empty").unwrap().size, 0);
    assert_eq!(s.seek(fd, 0, SeekWhence::End).unwrap(), 0);
}

#[test]
fn state_hash_changes_on_fd_offset_advance() {
    let mut s = fs_with("/foo", b"abc");
    let fd = s.open_fd("/foo").unwrap();
    let h0 = s.state_hash();
    let _ = s.read_at(fd, 1).unwrap();
    let h1 = s.state_hash();
    assert_ne!(h0, h1);
}

#[test]
fn state_hash_changes_on_open_close_pair() {
    let mut s = fs_with("/foo", b"x");
    let h0 = s.state_hash();
    let fd = s.open_fd("/foo").unwrap();
    let h1 = s.state_hash();
    assert_ne!(h0, h1);
    s.close_fd(fd).unwrap();
    let h2 = s.state_hash();
    // Pins the never-recycle property: next_fd advance is
    // observable even after the fd is closed.
    assert_ne!(h0, h2);
    assert_ne!(h1, h2);
}

#[test]
fn empty_store_is_empty() {
    let s = FsStore::new();
    assert!(s.is_empty());
    assert_eq!(s.blob_count(), 0);
    assert_eq!(s.open_fd_count(), 0);
}

#[test]
fn store_with_blob_is_not_empty() {
    let s = fs_with("/foo", b"x");
    assert!(!s.is_empty());
    assert_eq!(s.blob_count(), 1);
}

fn dir_entry(name: &str, is_directory: bool) -> DirEntry {
    DirEntry {
        name: name.to_owned(),
        is_directory,
    }
}

#[test]
fn open_dir_returns_distinct_fds_sharing_file_allocator() {
    let mut s = FsStore::new();
    s.register_blob("/foo".into(), b"x".to_vec()).unwrap();
    let file_fd = s.open_fd("/foo").unwrap();
    let dir_fd = s.open_dir(vec![dir_entry("a", false)]).unwrap();
    assert_eq!(dir_fd, file_fd + 1);
    assert_ne!(dir_fd, file_fd);
}

#[test]
fn read_dir_entry_walks_in_registration_order() {
    let mut s = FsStore::new();
    let entries = vec![
        dir_entry("a.xml", false),
        dir_entry("b.xml", false),
        dir_entry("sub", true),
    ];
    let fd = s.open_dir(entries).unwrap();
    let e0 = s.read_dir_entry(fd).unwrap().unwrap();
    assert_eq!(e0.name, "a.xml");
    assert!(!e0.is_directory);
    let e1 = s.read_dir_entry(fd).unwrap().unwrap();
    assert_eq!(e1.name, "b.xml");
    let e2 = s.read_dir_entry(fd).unwrap().unwrap();
    assert_eq!(e2.name, "sub");
    assert!(e2.is_directory);
    assert!(s.read_dir_entry(fd).unwrap().is_none());
    assert!(s.read_dir_entry(fd).unwrap().is_none());
}

#[test]
fn read_dir_entry_unknown_fd_is_unknown_dir() {
    let mut s = FsStore::new();
    assert_eq!(s.read_dir_entry(0xDEAD_BEEF), Err(FsError::UnknownDir));
    s.register_blob("/foo".into(), b"x".to_vec()).unwrap();
    let file_fd = s.open_fd("/foo").unwrap();
    assert_eq!(s.read_dir_entry(file_fd), Err(FsError::UnknownDir));
}

#[test]
fn close_dir_releases_fd_and_subsequent_ops_fail() {
    let mut s = FsStore::new();
    let fd = s.open_dir(vec![dir_entry("a", false)]).unwrap();
    assert!(s.close_dir(fd).is_ok());
    assert_eq!(s.close_dir(fd), Err(FsError::UnknownDir));
    assert_eq!(s.read_dir_entry(fd), Err(FsError::UnknownDir));
}

#[test]
fn close_dir_rejects_file_fd() {
    let mut s = FsStore::new();
    s.register_blob("/foo".into(), b"x".to_vec()).unwrap();
    let file_fd = s.open_fd("/foo").unwrap();
    assert_eq!(s.close_dir(file_fd), Err(FsError::UnknownDir));
    assert!(s.close_fd(file_fd).is_ok());
}

#[test]
fn close_fd_rejects_dir_fd() {
    let mut s = FsStore::new();
    let dir_fd = s.open_dir(vec![dir_entry("a", false)]).unwrap();
    assert_eq!(s.close_fd(dir_fd), Err(FsError::UnknownFd));
    assert!(s.close_dir(dir_fd).is_ok());
}

#[test]
fn open_dir_with_empty_entries_immediately_eofs() {
    let mut s = FsStore::new();
    let fd = s.open_dir(Vec::new()).unwrap();
    assert!(s.read_dir_entry(fd).unwrap().is_none());
}

#[test]
fn state_hash_changes_on_dir_open_and_advance() {
    let mut s = FsStore::new();
    let h0 = s.state_hash();
    let fd = s
        .open_dir(vec![dir_entry("a", false), dir_entry("b", false)])
        .unwrap();
    let h1 = s.state_hash();
    assert_ne!(h0, h1, "open_dir must contribute to state_hash");
    let _ = s.read_dir_entry(fd).unwrap();
    let h2 = s.state_hash();
    assert_ne!(h1, h2, "advancing the cursor must contribute");
    s.close_dir(fd).unwrap();
    let h3 = s.state_hash();
    assert_ne!(h2, h3);
    assert_ne!(h0, h3);
}

impl FsStore {
    /// Fast-forward the fd allocator to provoke
    /// [`FsError::FdExhausted`] without looping `open_fd` ~4 billion
    /// times.
    pub(crate) fn force_next_fd_for_test(&mut self, value: u32) {
        self.next_fd = value;
    }
}
