//! CellGov held to the `lv2_fs_unmounted_paths` and
//! `lv2_error_codes_fs_more` captures: what `sys_fs_open` and
//! `sys_fs_opendir` answer for a path on a root no device is mounted
//! at, and how many files a process may hold open.
//!
//! The chmod, write-after-chmod and rename cases of
//! `lv2_error_codes_fs_more` create and change files; CellGov's file
//! system is read-only, so they are not compared.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::syscall;

use super::common::{host_mounts, PathRuntime, TempMountDir};
use crate::fs_store::FsMount;
use crate::host::test_support::console_payload;
use crate::host::Lv2Host;
use crate::request::classify;
use crate::Lv2Dispatch;

/// `lv2_fs_unmounted_paths` probe paths, in payload order, and whether
/// each is an opendir.
const PATH_CASES: [(&str, bool); 14] = [
    ("/dev_hdd0/cgov_missing", false),
    ("/dev_usb000/cgov_missing", false),
    ("/dev_usb007/cgov_x", false),
    ("/dev_cgov/cgov_x", false),
    ("/cgov_nodev/cgov_x", false),
    ("/app_home/cgov_x", false),
    ("/host_root/cgov_x", false),
    ("cgov_relative", false),
    ("/dev_hdd1/cgov_x", false),
    ("/cgov_nodev", true),
    ("/dev_cgov", true),
    ("/app_home", true),
    ("/", false),
    ("/", true),
];

/// Cases CellGov answers otherwise, by name:
/// - 1: the reference console has a USB stick in port 0; CellGov models
///   no USB device, so `/dev_usb000` is unmounted here.
/// - 12, 13: CellGov mounts no `/`; the console answers `CELL_EISDIR`
///   to opening it and lists it.
const NAMED_DIFFERENCES: [usize; 3] = [1, 12, 13];

const PATH_AT: u32 = 0x1_0000;
const FD_WORD: u32 = 0x2_0000;

fn words(probe: &str) -> Vec<u64> {
    console_payload(probe)
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().unwrap()))
        .collect()
}

/// A host with `/dev_hdd0` mounted on an empty directory, the probe's
/// only mounted device besides the process-relative roots.
fn host(dir: &TempMountDir) -> Lv2Host {
    dir.write("hdd0/keep", b"");
    let mut host = Lv2Host::new();
    host_mounts(&mut host)
        .add(FsMount::new("/dev_hdd0", dir.path.join("hdd0")).expect("valid mount"))
        .expect("registration");
    host
}

/// One `sys_fs_open` (read-only) or `sys_fs_opendir` of `path`; the
/// return code and, on success, the fd it wrote.
fn open(host: &mut Lv2Host, path: &str, dir: bool) -> (u64, Option<u32>) {
    let mut bytes = path.as_bytes().to_vec();
    bytes.push(0);
    let mut mem = PathRuntime::empty(0x3_0000)
        .reserve(0, 0x1000)
        .write(PATH_AT, &bytes);
    let request = if dir {
        classify(
            syscall::FS_OPENDIR,
            &[u64::from(PATH_AT), u64::from(FD_WORD), 0, 0, 0, 0, 0, 0],
        )
    } else {
        classify(
            syscall::FS_OPEN,
            &[u64::from(PATH_AT), 0, u64::from(FD_WORD), 0, 0, 0, 0, 0],
        )
    };
    let Lv2Dispatch::Immediate { code, effects } = host.dispatch(request, UnitId::new(0), &mem)
    else {
        panic!("open of {path}: expected Immediate");
    };
    for effect in effects {
        if let Effect::SharedWriteIntent { range, bytes, .. } = effect {
            let at = range.start().raw() as usize;
            mem.bytes[at..at + bytes.bytes().len()].copy_from_slice(bytes.bytes());
        }
    }
    let at = FD_WORD as usize;
    let fd = (code == 0).then(|| u32::from_be_bytes(mem.bytes[at..at + 4].try_into().unwrap()));
    (code, fd)
}

#[test]
fn open_and_opendir_answer_the_console_s_code_on_each_root() {
    let console = words("lv2_fs_unmounted_paths");
    assert_eq!(console.len(), PATH_CASES.len(), "payload size");
    let dir = TempMountDir::new("fs_unmounted_paths");
    let mut compared = 0;
    for (i, &(path, is_dir)) in PATH_CASES.iter().enumerate() {
        if NAMED_DIFFERENCES.contains(&i) {
            continue;
        }
        let (ours, _) = open(&mut host(&dir), path, is_dir);
        assert_eq!(
            ours as u32,
            console[i] as u32,
            "case {i}: {} {path}",
            if is_dir { "opendir" } else { "open" }
        );
        compared += 1;
    }
    assert_eq!(compared, PATH_CASES.len() - NAMED_DIFFERENCES.len());
}

#[test]
fn the_named_differences_still_differ() {
    let console = words("lv2_fs_unmounted_paths");
    let dir = TempMountDir::new("fs_unmounted_named");
    for i in NAMED_DIFFERENCES {
        let (path, is_dir) = PATH_CASES[i];
        let (ours, _) = open(&mut host(&dir), path, is_dir);
        assert_ne!(
            ours as u32, console[i] as u32,
            "case {i} ({path}) now agrees; drop it from NAMED_DIFFERENCES"
        );
    }
}

/// `lv2_error_codes_fs_more` cases 0 to 2: the unmounted devices.
#[test]
fn open_on_an_unmounted_device_answers_the_console_s_code() {
    let console = words("lv2_error_codes_fs_more");
    let dir = TempMountDir::new("fs_more_unmounted");
    for (i, (path, is_dir)) in [
        ("/dev_usb007/cgov_x", false),
        ("/dev_usb007", true),
        ("/dev_bdvd/cgov_x", false),
    ]
    .into_iter()
    .enumerate()
    {
        let (ours, _) = open(&mut host(&dir), path, is_dir);
        assert_eq!(ours as u32, console[i] as u32, "case {i}: {path}");
    }
}

/// `lv2_error_codes_fs_more` cases 6 to 9: opens of one file until the
/// first refusal, that refusal, and the first and last fd handed out.
#[test]
fn the_open_file_limit_matches_the_console() {
    let console = words("lv2_error_codes_fs_more");
    let dir = TempMountDir::new("fs_more_limit");
    dir.write("hdd0/tmp/file", b"");
    let mut host = host(&dir);
    let mut fds = Vec::new();
    let refusal = loop {
        match open(&mut host, "/dev_hdd0/tmp/file", false) {
            (0, Some(fd)) => fds.push(fd),
            (code, _) => break code,
        }
        assert!(fds.len() <= 300, "the probe stopped at 300 opens");
    };
    assert_eq!(fds.len() as u64, console[6], "opens before the refusal");
    assert_eq!(refusal as u32, console[7] as u32, "the refusal");
    assert_eq!(u64::from(fds[0]), console[8], "first fd");
    assert_eq!(u64::from(*fds.last().unwrap()), console[9], "last fd");
}
