//! CellGov held to the `lv2_fs_constants` capture: the open-flag bits a
//! read-only open takes, the path and name limits, and the bytes the
//! stat and readdir records fill. The probe's calls run through dispatch
//! against a mounted host directory holding the probe's file `f`, and
//! each payload word is rebuilt the way
//! `tests/micro/lv2_fs_constants/ppu/main.c` builds it.
//!
//! The write-side words (creates, truncates, appends) have no
//! counterpart in CellGov's read-only model and are not compared here.
//! One answer differs by name: an `O_MSELF` open of a plain file. The
//! `st_mode` words (slots 38 and 61) differ as well;
//! `st_mode_is_the_created_bits_on_the_console_and_read_only_in_cellgov`
//! holds them.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::syscall;

use super::common::{host_mounts, PathRuntime, TempMountDir};
use crate::fs_store::FsMount;
use crate::host::test_support::console_payload;
use crate::host::Lv2Host;
use crate::request::classify;
use crate::Lv2Dispatch;

const WORDS: usize = 64;
const SENTINEL: u64 = 0xEEEE_EEEE_EEEE_EEEE;

/// Guest addresses: the path buffer, then the probe's output buffers.
const PATH: u32 = 0x1_0000;
const OUT_WORD: u32 = 0x2_0000;
const FD_WORD: u32 = 0x2_0010;
const BUF: u32 = 0x2_0100;
const BUF_LEN: usize = 0x200;
const WXYZ: u32 = 0x2_0500;

const DIR_D: &str = "/dev_hdd0/tmp/cgov_fsc_dir";
const FILE_F: &str = "/dev_hdd0/tmp/cgov_fsc_dir/f";

/// The probe's extra open-flag bits, in payload order (slots 18-29).
const EXTRA_BITS: [u64; 12] = [
    0x4, 0x8, 0x10, 0x20, 0x100, 0x800, 0x1000, 0x2000, 0x4000, 0x8000, 0x1_0000, 0x2_0000,
];

struct Guest {
    host: Lv2Host,
    mem: PathRuntime,
    _dir: TempMountDir,
}

impl Guest {
    fn new() -> Self {
        let dir = TempMountDir::new("fs_constants_console");
        dir.write("cgov_fsc_dir/f", b"");
        let mut host = Lv2Host::new();
        host_mounts(&mut host)
            .add(FsMount::new("/dev_hdd0/tmp", dir.path.clone()).expect("valid mount"))
            .expect("registration");
        let mem = PathRuntime::empty(0x4_0000)
            .reserve(0, 0x1000)
            .write(WXYZ, b"WXYZ");
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

    /// Put `path` and its NUL at PATH.
    fn path(&mut self, path: &str) -> u64 {
        let at = PATH as usize;
        self.mem.bytes[at..at + path.len()].copy_from_slice(path.as_bytes());
        self.mem.bytes[at + path.len()] = 0;
        u64::from(PATH)
    }

    fn fill(&mut self) {
        self.mem.bytes[BUF as usize..BUF as usize + BUF_LEN].fill(0xEE);
    }

    /// The index of the last byte of the buffer a call changed, plus
    /// one.
    fn written(&self) -> u64 {
        let buf = &self.mem.bytes[BUF as usize..BUF as usize + BUF_LEN];
        buf.iter().rposition(|&b| b != 0xEE).map_or(0, |i| i + 1) as u64
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

    /// Open `path`: the fd when it opened, and the return code.
    fn open(&mut self, path: &str, flags: u64, mode: u64) -> (Option<u64>, u64) {
        let p = self.path(path);
        let rc = self.sc(syscall::FS_OPEN, [p, flags, u64::from(FD_WORD), mode]);
        let fd = (rc == 0).then(|| self.u32_at(FD_WORD));
        (fd, rc)
    }

    fn close(&mut self, fd: Option<u64>) {
        if let Some(fd) = fd {
            assert_eq!(self.sc(syscall::FS_CLOSE, [fd, 0, 0, 0]), 0);
        }
    }

    fn stat(&mut self, path: &str) -> u64 {
        self.fill();
        let p = self.path(path);
        self.sc(syscall::FS_STAT, [p, u64::from(BUF), 0, 0])
    }
}

/// The probe's path of `len` bytes: D, then components of 'a' no longer
/// than 200 bytes.
fn long_path(len: usize) -> String {
    let mut p = DIR_D.to_string();
    while p.len() < len {
        p.push('/');
        let run = (len - p.len()).min(200);
        p.push_str(&"a".repeat(run));
    }
    if p.ends_with('/') {
        p.pop();
        p.push('a');
    }
    p
}

/// CellGov's answer to the probe's read-side words; write-side slots
/// keep the sentinel.
fn cellgov_words() -> Vec<u64> {
    let mut g = Guest::new();
    let mut w = vec![SENTINEL; WORDS];

    let (fd, rc) = g.open(FILE_F, 0, 0);
    w[3] = rc;
    if let Some(fd) = fd {
        w[4] = g.sc(
            syscall::FS_WRITE,
            [fd, u64::from(WXYZ), 1, u64::from(OUT_WORD)],
        );
    }
    g.close(fd);
    let (fd, rc) = g.open(FILE_F, 3, 0);
    w[8] = rc;
    g.close(fd);
    let (fd, rc) = g.open(FILE_F, 0x40 | 0x80, 0x180);
    w[9] = rc;
    g.close(fd);
    for (i, bit) in EXTRA_BITS.into_iter().enumerate() {
        let (fd, rc) = g.open(FILE_F, bit, 0);
        w[18 + i] = rc;
        g.close(fd);
    }

    for (i, len) in [1021, 1022, 1023, 1024].into_iter().enumerate() {
        w[30 + i] = g.stat(&long_path(len));
    }
    w[34] = g.stat(&format!("{DIR_D}/{}", "n".repeat(255)));
    w[35] = g.stat(&format!("{DIR_D}/{}", "n".repeat(256)));

    w[36] = g.stat(FILE_F);
    w[37] = g.written();
    w[40] = g.u64_at(BUF + 44);
    let (fd, _) = g.open(FILE_F, 0, 0);
    let fd = fd.expect("f opens");
    g.fill();
    w[41] = g.sc(syscall::FS_FSTAT, [fd, u64::from(BUF), 0, 0]);
    w[42] = g.written();
    g.close(Some(fd));
    let d = g.path(DIR_D);
    w[43] = g.sc(syscall::FS_OPENDIR, [d, u64::from(FD_WORD), 0, 0]);
    let dir_fd = g.u32_at(FD_WORD);
    for i in 0..4 {
        g.fill();
        g.mem.bytes[OUT_WORD as usize..OUT_WORD as usize + 8]
            .copy_from_slice(&SENTINEL.to_be_bytes());
        let rc = g.sc(
            syscall::FS_READDIR,
            [dir_fd, u64::from(BUF), u64::from(OUT_WORD), 0],
        );
        let nread = g.u64_at(OUT_WORD);
        w[44 + 3 * i] = (rc << 32) | (nread & 0xFFFF_FFFF);
        w[45 + 3 * i] = g.written();
        let b = BUF as usize;
        w[46 + 3 * i] = (u64::from(g.mem.bytes[b]) << 8) | u64::from(g.mem.bytes[b + 1]);
        if rc != 0 || nread == 0 {
            break;
        }
    }
    w[56] = g.sc(syscall::FS_CLOSEDIR, [dir_fd, 0, 0, 0]);
    w[60] = g.stat(DIR_D);
    w
}

fn console_words() -> Vec<u64> {
    let payload = console_payload("lv2_fs_constants");
    assert_eq!(payload.len(), WORDS * 8, "payload size");
    payload
        .chunks_exact(8)
        .map(|c| u64::from_be_bytes(c.try_into().unwrap()))
        .collect()
}

/// The read-side slots, compared word for word. Slot 24, the
/// `O_MSELF` open, has its own test.
const COMPARED: [usize; 41] = [
    3, 4, 8, 9, 18, 19, 20, 21, 22, 23, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 40, 41,
    42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 60,
];

/// The `lv2_fs_path_limit` and `lv2_fs_path_limit_mount` captures:
/// `sys_fs_stat` of a path naming nothing, built as the probes build
/// it, at each length they send.
#[test]
fn a_long_path_is_taken_or_too_long_as_on_the_console() {
    let words = |probe: &str| -> Vec<u64> {
        console_payload(probe)
            .chunks_exact(8)
            .map(|c| u64::from_be_bytes(c.try_into().unwrap()))
            .collect()
    };
    let path_of = |base: &str, len: usize| {
        let mut p = base.to_string();
        while p.len() < len {
            p.push('/');
            let run = (len - p.len()).min(200);
            p.push_str(&"a".repeat(run));
        }
        if p.ends_with('/') {
            p.pop();
            p.push('a');
        }
        p
    };
    let mut cases = Vec::new();
    let lens = [
        1024, 1025, 1026, 1027, 1028, 1032, 1040, 1056, 1088, 1152, 1280, 1536, 2047, 2048, 4095,
        4096,
    ];
    let wide = words("lv2_fs_path_limit");
    for (i, len) in lens.into_iter().enumerate() {
        cases.push(("/dev_hdd0/tmp/cgov_fspl_missing", len, wide[i]));
    }
    let mount = words("lv2_fs_path_limit_mount");
    for (b, base) in [
        "/dev_hdd0/tmp/cgov_fspl_missing",
        "/dev_flash/cgov_fspl_missing",
    ]
    .into_iter()
    .enumerate()
    {
        for i in 0..8 {
            cases.push((base, 1033 + i, mount[b * 8 + i]));
        }
    }
    let mut g = Guest::new();
    let differing: Vec<String> = cases
        .into_iter()
        .filter_map(|(base, len, console)| {
            let got = g.stat(&path_of(base, len));
            (got != console)
                .then(|| format!("{base} {len}: console {console:#x}, CellGov {got:#x}"))
        })
        .collect();
    assert!(differing.is_empty(), "differs:\n{}", differing.join("\n"));
}

/// The console refuses an `O_MSELF` open of a file that is not an MSELF
/// archive; CellGov reads no MSELF header and opens it.
#[test]
fn an_mself_open_of_a_plain_file_is_refused_on_the_console_and_opens_in_cellgov() {
    use cellgov_ps3_abi::lv2::{errno, fs::CELL_FS_O_MSELF};
    let console = console_words();
    let cellgov = cellgov_words();
    assert_eq!(EXTRA_BITS[24 - 18], u64::from(CELL_FS_O_MSELF));
    assert_eq!(console[24], u64::from(errno::CELL_ENOTMSELF));
    assert_eq!(cellgov[24], 0);
}

#[test]
fn cellgov_answers_the_read_side_of_the_constants_probe_as_the_console_does() {
    let console = console_words();
    let cellgov = cellgov_words();
    let differing: Vec<String> = COMPARED
        .into_iter()
        .filter(|&s| cellgov[s] != console[s])
        .map(|s| {
            format!(
                "slot {s}: console {:#x}, CellGov {:#x}",
                console[s], cellgov[s]
            )
        })
        .collect();
    assert!(differing.is_empty(), "differs:\n{}", differing.join("\n"));
}
