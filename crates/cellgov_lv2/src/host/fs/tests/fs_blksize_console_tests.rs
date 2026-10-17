//! CellGov held to the `lv2_fs_blksize` capture: the block size
//! `sys_fs_stat` answers for a file and a directory on each device the
//! console had mounted, a game disc among them. The probe's
//! `/app_home` paths refused on the console (a runner launch mounts no
//! app_home), so they settle nothing and are not compared.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::syscall;

use super::common::{host_mounts, PathRuntime, TempMountDir};
use crate::fs_store::FsMount;
use crate::host::test_support::console_payload;
use crate::host::Lv2Host;
use crate::request::classify;
use crate::Lv2Dispatch;

/// The probe's paths, in payload order.
const PROBE_PATHS: [&str; 8] = [
    "/dev_hdd0/game/CGOV00001/USRDIR/EBOOT.BIN",
    "/app_home/USRDIR/EBOOT.BIN",
    "/app_home",
    "/dev_flash/sys/external/liblv2.sprx",
    "/dev_flash",
    "/dev_bdvd/PS3_GAME/PARAM.SFO",
    "/dev_bdvd",
    "/dev_usb000",
];

/// The records whose stat the console answered.
const MOUNTED: [usize; 6] = [0, 3, 4, 5, 6, 7];

/// The disc's records: a file and the disc's root.
const DISC: [usize; 2] = [5, 6];

const PATH_AT: u32 = 0x1_0000;
const STAT_BUF: u32 = 0x2_0000;

/// One stat answer: r3, `st_mode`, `st_size` and `st_blksize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stat {
    rc: u64,
    mode: u64,
    size: u64,
    blksize: u64,
}

/// The console's answer per record. The probe read the record's words
/// at bytes 0, 40 and 48; `st_size` sits at byte 36 and `st_blksize`
/// at byte 44. The probe never read bytes 36 to 39, the high half of
/// `st_size`, which is zero for every file it stats (each under 4 GiB);
/// `st_blksize` spans two of its words.
fn console() -> Vec<Stat> {
    let payload = console_payload("lv2_fs_blksize");
    assert_eq!(payload.len(), PROBE_PATHS.len() * 32, "payload size");
    payload
        .chunks_exact(32)
        .map(|r| {
            let word = |k: usize| u64::from_be_bytes(r[k * 8..k * 8 + 8].try_into().unwrap());
            Stat {
                rc: word(0),
                mode: word(1),
                size: word(2) >> 32,
                blksize: (word(2) << 32) | (word(3) >> 32),
            }
        })
        .collect()
}

/// CellGov's answer for `path` with the probe's devices mounted.
fn cellgov(path: &str) -> Stat {
    let dir = TempMountDir::new("fs_blksize_console");
    dir.write("game/CGOV00001/USRDIR/EBOOT.BIN", &[0u8; 0x1600]);
    dir.write("flash/sys/external/liblv2.sprx", b"sprx");
    dir.write("usb/keep", b"");
    dir.write("disc/PS3_GAME/PARAM.SFO", &[0u8; 0x654]);
    let mut host = Lv2Host::new();
    for (mount, sub) in [
        ("/dev_hdd0/game", "game"),
        ("/dev_flash", "flash"),
        ("/dev_usb000", "usb"),
        ("/dev_bdvd", "disc"),
    ] {
        host_mounts(&mut host)
            .add(FsMount::new(mount, dir.path.join(sub)).expect("valid mount"))
            .expect("registration");
    }
    let mut bytes = path.as_bytes().to_vec();
    bytes.push(0);
    let mut mem = PathRuntime::empty(0x3_0000)
        .reserve(0, 0x1000)
        .write(PATH_AT, &bytes);
    let request = classify(
        syscall::FS_STAT,
        &[u64::from(PATH_AT), u64::from(STAT_BUF), 0, 0, 0, 0, 0, 0],
    );
    let Lv2Dispatch::Immediate { code, effects } = host.dispatch(request, UnitId::new(0), &mem)
    else {
        panic!("stat of {path}: expected Immediate");
    };
    for effect in effects {
        if let Effect::SharedWriteIntent { range, bytes, .. } = effect {
            let at = range.start().raw() as usize;
            mem.bytes[at..at + bytes.bytes().len()].copy_from_slice(bytes.bytes());
        }
    }
    let at = STAT_BUF as usize;
    let u64_at =
        |off: usize| u64::from_be_bytes(mem.bytes[at + off..at + off + 8].try_into().unwrap());
    Stat {
        rc: code,
        mode: u64::from(u32::from_be_bytes(
            mem.bytes[at..at + 4].try_into().unwrap(),
        )),
        size: u64_at(36),
        blksize: u64_at(44),
    }
}

#[test]
fn stat_answers_the_console_s_block_size_on_each_mounted_device() {
    let console = console();
    for i in MOUNTED {
        assert_eq!(console[i].rc, 0, "{}: the console's stat", PROBE_PATHS[i]);
        let ours = cellgov(PROBE_PATHS[i]);
        assert_eq!(
            (ours.rc, ours.blksize),
            (console[i].rc, console[i].blksize),
            "{}: (r3, st_blksize)",
            PROBE_PATHS[i]
        );
    }
}

/// On the read-only disc the whole record agrees: the read-only mode
/// bits CellGov stands in with are the disc's own, and a directory's
/// size is one disc block.
#[test]
fn stat_on_the_disc_answers_the_console_s_whole_record() {
    let console = console();
    for i in DISC {
        assert_eq!(cellgov(PROBE_PATHS[i]), console[i], "{}", PROBE_PATHS[i]);
    }
}

/// `fstat` of a file opened on the disc answers the disc's block, as
/// `stat` of its path does.
#[test]
fn fstat_of_a_disc_file_answers_the_disc_s_block() {
    let dir = TempMountDir::new("fs_blksize_fstat");
    dir.write("disc/PS3_GAME/PARAM.SFO", &[0u8; 0x654]);
    let mut host = Lv2Host::new();
    host_mounts(&mut host)
        .add(FsMount::new("/dev_bdvd", dir.path.join("disc")).expect("valid mount"))
        .expect("registration");
    let mut bytes = PROBE_PATHS[5].as_bytes().to_vec();
    bytes.push(0);
    const FD_WORD: u32 = 0x2_1000;
    let mut mem = PathRuntime::empty(0x3_0000)
        .reserve(0, 0x1000)
        .write(PATH_AT, &bytes);
    let call = |host: &mut Lv2Host, mem: &mut PathRuntime, number: u64, args: [u64; 4]| {
        let [a, b, c, d] = args;
        let Lv2Dispatch::Immediate { code, effects } = host.dispatch(
            classify(number, &[a, b, c, d, 0, 0, 0, 0]),
            UnitId::new(0),
            &*mem,
        ) else {
            panic!("syscall {number}: expected Immediate");
        };
        for effect in effects {
            if let Effect::SharedWriteIntent { range, bytes, .. } = effect {
                let at = range.start().raw() as usize;
                mem.bytes[at..at + bytes.bytes().len()].copy_from_slice(bytes.bytes());
            }
        }
        code
    };
    let open = call(
        &mut host,
        &mut mem,
        syscall::FS_OPEN,
        [u64::from(PATH_AT), 0, u64::from(FD_WORD), 0],
    );
    assert_eq!(open, 0, "the disc file opens read-only");
    let fd = u32::from_be_bytes(
        mem.bytes[FD_WORD as usize..FD_WORD as usize + 4]
            .try_into()
            .unwrap(),
    );
    let rc = call(
        &mut host,
        &mut mem,
        syscall::FS_FSTAT,
        [u64::from(fd), u64::from(STAT_BUF), 0, 0],
    );
    assert_eq!(rc, 0);
    let at = STAT_BUF as usize + 44;
    assert_eq!(
        u64::from_be_bytes(mem.bytes[at..at + 8].try_into().unwrap()),
        console()[5].blksize
    );
}
