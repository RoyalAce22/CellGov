//! CellGov held to the `lv2_fs_answers` capture: the probe's calls run
//! through dispatch against a mounted host directory holding the
//! probe's four files, and each payload word is rebuilt the way
//! `tests/micro/lv2_fs_answers/ppu/main.c` builds it.
//!
//! Three answers differ by name, each asserted on its own:
//!
//! - the console opens a file `O_RDWR | O_CREAT`, where CellGov's
//!   read-only model refuses with `CELL_EROFS` (CellGov's file `w` is
//!   opened read-only in its place, to carry the stat, lseek and read
//!   words);
//! - the console's `st_mode` carries the bits a file was created with,
//!   where CellGov reports a read-only regular file;
//! - after `.` and `..` the console lists entries in creation order,
//!   where CellGov lists them in byte order.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::fs::{CELL_FS_O_CREAT, CELL_FS_O_RDONLY, CELL_FS_O_RDWR};
use cellgov_ps3_abi::lv2::syscall;

use super::common::{host_mounts, PathRuntime, TempMountDir};
use crate::fs_store::FsMount;
use crate::host::test_support::console_payload;
use crate::host::Lv2Host;
use crate::request::classify;
use crate::Lv2Dispatch;

const WORDS: usize = 64;
const SENTINEL: u64 = 0xEEEE_EEEE_EEEE_EEEE;
const BAD_FD: u64 = 0x7FFF;
const UNMAPPED: u64 = 0x100;

const O_RDONLY: u64 = CELL_FS_O_RDONLY as u64;
const O_RDWR: u64 = CELL_FS_O_RDWR as u64;
const O_CREAT: u64 = CELL_FS_O_CREAT as u64;
const MODE_FILE: u64 = 0x180;
const SEEK_SET: u64 = 0;
const SEEK_CUR: u64 = 1;
const SEEK_END: u64 = 2;

/// Guest addresses: the paths, then the probe's output buffers.
const PATHS: u32 = 0x1_0000;
const OUT_WORD: u32 = 0x2_0000;
const FD_WORD: u32 = 0x2_0010;
const STAT_BUF: u32 = 0x2_0100;
const DIRENT: u32 = 0x2_0200;
const BYTES: u32 = 0x2_0400;
const WXYZ: u32 = 0x2_0500;

/// The probe's directory and its files, in the order the paths sit in
/// guest memory.
const NAMES: [&str; 5] = [
    "/dev_hdd0/tmp/cgov_fsans_dir",
    "/dev_hdd0/tmp/cgov_fsans_dir/a",
    "/dev_hdd0/tmp/cgov_fsans_dir/b",
    "/dev_hdd0/tmp/cgov_fsans_dir/c",
    "/dev_hdd0/tmp/cgov_fsans_dir/w",
];

/// The probe's calls against a host and a byte sandbox standing in for
/// guest memory; every write a call returns lands in the sandbox.
struct Guest {
    host: Lv2Host,
    mem: PathRuntime,
    _dir: TempMountDir,
}

impl Guest {
    fn new() -> Self {
        let dir = TempMountDir::new("fs_answers_console");
        for name in ["a", "b", "c"] {
            dir.write(&format!("cgov_fsans_dir/{name}"), b"");
        }
        dir.write("cgov_fsans_dir/w", b"WXYZ");
        let mut host = Lv2Host::new();
        host_mounts(&mut host)
            .add(FsMount::new("/dev_hdd0/tmp", dir.path.clone()).expect("valid mount"))
            .expect("registration");
        let mut mem = PathRuntime::empty(0x4_0000).reserve(0, 0x1000);
        for (i, name) in NAMES.iter().enumerate() {
            let mut bytes = name.as_bytes().to_vec();
            bytes.push(0);
            mem = mem.write(path(i), &bytes);
        }
        mem = mem.write(WXYZ, b"WXYZ");
        Self {
            host,
            mem,
            _dir: dir,
        }
    }

    fn sc(&mut self, number: u64, args: [u64; 4]) -> u64 {
        let [a, b, c, d] = args;
        let request = classify(number, &[a, b, c, d, 0, 0, 0, 0]);
        match self.host.dispatch(request, UnitId::new(0), &self.mem) {
            Lv2Dispatch::Immediate { code, effects } => {
                for effect in effects {
                    if let Effect::SharedWriteIntent { range, bytes, .. } = effect {
                        let at = range.start().raw() as usize;
                        self.mem.bytes[at..at + bytes.bytes().len()].copy_from_slice(bytes.bytes());
                    }
                }
                code
            }
            other => panic!("syscall {number}: expected Immediate, got {other:?}"),
        }
    }

    fn fill(&mut self, at: u32, len: usize, byte: u8) {
        self.mem.bytes[at as usize..at as usize + len].fill(byte);
    }

    fn u32_at(&self, at: u32) -> u64 {
        let at = at as usize;
        u64::from(u32::from_be_bytes(
            self.mem.bytes[at..at + 4].try_into().unwrap(),
        ))
    }

    fn u64_at(&self, at: u32) -> u64 {
        let at = at as usize;
        u64::from_be_bytes(self.mem.bytes[at..at + 8].try_into().unwrap())
    }

    fn set_out(&mut self) {
        self.mem.bytes[OUT_WORD as usize..OUT_WORD as usize + 8]
            .copy_from_slice(&SENTINEL.to_be_bytes());
    }

    /// Open `path`; the fd as the probe records it, and the return code.
    fn open(&mut self, path: u32, flags: u64, mode: u64) -> (i64, u64) {
        self.fill(FD_WORD, 4, 0xFF);
        let rc = self.sc(
            syscall::FS_OPEN,
            [u64::from(path), flags, u64::from(FD_WORD), mode],
        );
        let fd = if rc == 0 {
            self.u32_at(FD_WORD) as i64
        } else {
            -1
        };
        (fd, rc)
    }

    fn stat_words(&self, words: &mut [u64], slot: usize) {
        words[slot] = self.u32_at(STAT_BUF);
        words[slot + 1] = self.u64_at(STAT_BUF + 40);
        words[slot + 2] = self.u64_at(STAT_BUF + 48);
    }

    fn seek(&mut self, words: &mut [u64], fd: u64, offset: i64, whence: u64, slot: usize) {
        self.set_out();
        words[slot] = self.sc(
            syscall::FS_LSEEK,
            [fd, offset as u64, whence, u64::from(OUT_WORD)],
        );
        words[slot + 1] = self.u64_at(OUT_WORD);
    }
}

fn path(i: usize) -> u32 {
    PATHS + 0x100 * i as u32
}

/// CellGov's answer to the probe, word for word, and the return code of
/// its `O_RDWR | O_CREAT` open, which the payload's word 3 holds.
fn cellgov_words() -> (Vec<u64>, u64) {
    let mut g = Guest::new();
    let mut w = vec![SENTINEL; WORDS];

    // The probe creates b, a and c, closing each, before its opens;
    // read-only opens stand in for the creates, so the fds the opens
    // read show whether a closed fd is handed out again.
    for i in [2, 1, 3] {
        let (fd, rc) = g.open(path(i), O_RDONLY, 0);
        assert_eq!(rc, 0, "open of {}", NAMES[i]);
        assert_eq!(g.sc(syscall::FS_CLOSE, [fd as u64, 0, 0, 0]), 0);
    }
    let (fd_a, _) = g.open(path(1), O_RDONLY, 0);
    let (fd_b, _) = g.open(path(2), O_RDONLY, 0);
    let (fd_c, _) = g.open(path(3), O_RDONLY, 0);
    w[0] = fd_a as u64;
    w[1] = fd_b as u64;
    w[2] = fd_c as u64;

    let (_, rdwr_rc) = g.open(path(4), O_RDWR | O_CREAT, MODE_FILE);
    let (fd_w, rc) = g.open(path(4), O_RDONLY, 0);
    w[3] = rc;
    w[4] = fd_w as u64;
    let fd_w = fd_w as u64;
    let fd_a = fd_a as u64;
    // The console's 4-byte write to its read-write `w` has no CellGov
    // counterpart; words 5 and 6 come from the capture's own premise
    // below.
    g.set_out();
    w[7] = g.sc(
        syscall::FS_WRITE,
        [fd_a, u64::from(WXYZ), 0, u64::from(OUT_WORD)],
    );
    w[8] = g.u64_at(OUT_WORD);
    g.set_out();
    w[9] = g.sc(
        syscall::FS_WRITE,
        [fd_a, u64::from(WXYZ), 4, u64::from(OUT_WORD)],
    );
    w[10] = g.u64_at(OUT_WORD);

    g.fill(STAT_BUF, 64, 0xEE);
    w[11] = g.sc(syscall::FS_FSTAT, [fd_w, u64::from(STAT_BUF), 0, 0]);
    g.stat_words(&mut w, 12);
    g.fill(STAT_BUF, 64, 0xEE);
    w[15] = g.sc(
        syscall::FS_STAT,
        [u64::from(path(4)), u64::from(STAT_BUF), 0, 0],
    );
    g.stat_words(&mut w, 16);
    g.fill(STAT_BUF, 64, 0xEE);
    w[19] = g.sc(
        syscall::FS_STAT,
        [u64::from(path(0)), u64::from(STAT_BUF), 0, 0],
    );
    g.stat_words(&mut w, 20);

    g.seek(&mut w, fd_w, 1, SEEK_SET, 23);
    g.seek(&mut w, fd_w, 1, SEEK_CUR, 25);
    g.seek(&mut w, fd_w, 0, SEEK_END, 27);
    g.seek(&mut w, fd_w, 10, SEEK_SET, 29);
    g.seek(&mut w, fd_w, -1, SEEK_SET, 31);
    g.set_out();
    w[33] = g.sc(syscall::FS_LSEEK, [fd_w, 0, 3, u64::from(OUT_WORD)]);

    g.set_out();
    w[34] = g.sc(syscall::FS_LSEEK, [fd_w, 0, SEEK_SET, u64::from(OUT_WORD)]);
    g.set_out();
    w[35] = g.sc(syscall::FS_READ, [fd_w, UNMAPPED, 2, u64::from(OUT_WORD)]);
    w[36] = g.u64_at(OUT_WORD);
    g.set_out();
    g.sc(syscall::FS_LSEEK, [fd_w, 0, SEEK_CUR, u64::from(OUT_WORD)]);
    w[37] = g.u64_at(OUT_WORD);
    g.fill(BYTES, 8, 0);
    g.set_out();
    w[38] = g.sc(
        syscall::FS_READ,
        [fd_w, u64::from(BYTES), 2, u64::from(OUT_WORD)],
    );
    w[39] = g.u64_at(OUT_WORD);
    w[40] =
        (u64::from(g.mem.bytes[BYTES as usize]) << 8) | u64::from(g.mem.bytes[BYTES as usize + 1]);

    g.fill(FD_WORD, 4, 0xFF);
    w[41] = g.sc(
        syscall::FS_OPENDIR,
        [u64::from(path(0)), u64::from(FD_WORD), 0, 0],
    );
    let dir_fd = g.u32_at(FD_WORD);
    w[42] = dir_fd;
    for i in 0..8 {
        g.fill(DIRENT, 264, 0);
        g.set_out();
        let rc = g.sc(
            syscall::FS_READDIR,
            [dir_fd, u64::from(DIRENT), u64::from(OUT_WORD), 0],
        );
        let nread = g.u64_at(OUT_WORD);
        w[43 + 2 * i] = (rc << 32) | (nread & 0xFFFF_FFFF);
        let d = DIRENT as usize;
        let n = usize::from(g.mem.bytes[d + 1]);
        let mut name = 0u64;
        for k in 0..6 {
            name = (name << 8)
                | if k < n {
                    u64::from(g.mem.bytes[d + 2 + k])
                } else {
                    0
                };
        }
        w[44 + 2 * i] = (u64::from(g.mem.bytes[d]) << 56) | ((n as u64) << 48) | name;
        if rc != 0 || nread == 0 {
            break;
        }
    }
    w[59] = g.sc(syscall::FS_CLOSEDIR, [dir_fd, 0, 0, 0]);
    w[60] = g.sc(syscall::FS_CLOSEDIR, [dir_fd, 0, 0, 0]);

    w[61] = g.sc(syscall::FS_READDIR, [BAD_FD, 0, 0, 0]);
    w[62] = g.sc(syscall::FS_READDIR, [BAD_FD, u64::from(DIRENT), 0, 0]);
    g.set_out();
    w[63] = g.sc(syscall::FS_READDIR, [BAD_FD, 0, u64::from(OUT_WORD), 0]);
    (w, rdwr_rc)
}

fn console_words() -> Vec<u64> {
    let payload = console_payload("lv2_fs_answers");
    assert_eq!(payload.len(), WORDS * 8, "payload size");
    payload
        .chunks_exact(8)
        .map(|c| u64::from_be_bytes(c.try_into().unwrap()))
        .collect()
}

/// The payload slots CellGov answers like the console.
const AGREE: [usize; 50] = [
    0, 1, 2, 4, 7, 8, 9, 10, 11, 13, 14, 15, 17, 18, 19, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
    31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 45, 47, 49, 51, 53, 55, 57, 58, 59, 60, 61,
    62,
];

#[test]
fn cellgov_answers_the_file_system_probe_as_the_console_does() {
    let console = console_words();
    let (cellgov, _) = cellgov_words();
    for slot in AGREE.into_iter().chain([63]) {
        assert_eq!(
            cellgov[slot], console[slot],
            "slot {slot}: console {:#x}, CellGov {:#x}",
            console[slot], cellgov[slot]
        );
    }
}

#[test]
fn a_read_write_open_is_the_read_only_models_one_refusal() {
    let console = console_words();
    let (_, rdwr_rc) = cellgov_words();
    assert_eq!(console[3], 0, "the console opens O_RDWR | O_CREAT");
    assert_eq!(console[5], 0, "and writes 4 bytes");
    assert_eq!(console[6], 4);
    assert_eq!(rdwr_rc, u64::from(errno::CELL_EROFS));
}

#[test]
fn st_mode_is_the_created_bits_on_the_console_and_read_only_in_cellgov() {
    let console = console_words();
    let (cellgov, _) = cellgov_words();
    assert_eq!(console[12], 0x8180, "S_IFREG | 0o600 on the console");
    assert_eq!(console[16], 0x8180);
    assert_eq!(cellgov[12], 0x8124, "S_IFREG | 0o444 in CellGov");
    assert_eq!(cellgov[16], 0x8124);
    assert_eq!(console[20], 0x41C0, "S_IFDIR | 0o700 on the console");
    assert_eq!(cellgov[20], 0x416D, "S_IFDIR | 0o555 in CellGov");
}

#[test]
fn readdir_lists_dot_entries_first_and_the_same_names_after() {
    let console = console_words();
    let (cellgov, _) = cellgov_words();
    let names = |w: &[u64]| -> Vec<u64> { (0..7).map(|i| w[44 + 2 * i]).collect() };
    let (c, g) = (names(&console), names(&cellgov));
    assert_eq!(c[..2], g[..2], "`.` and `..` lead both listings");
    assert_eq!(c[6], g[6], "both end with an empty entry");
    let mut c_rest = c[2..6].to_vec();
    let mut g_rest = g[2..6].to_vec();
    assert_ne!(
        c_rest, g_rest,
        "creation order on the console, byte order in CellGov"
    );
    c_rest.sort_unstable();
    g_rest.sort_unstable();
    assert_eq!(c_rest, g_rest, "the same four names");
}
