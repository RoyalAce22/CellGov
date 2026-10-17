//! PS3 `sys_fs` ABI constants: open flags, the `CellFsStat` wire
//! format, mode bits, and the maximum path length.
//!
//! Octal literals match the canonical PS3 open-flag spelling (using
//! hex risks silent transcription errors on the order-of-magnitude
//! nibble: `O_CREAT = 0o100 = 0x40`, not `0x4`).

/// The first fd the kernel hands out for file and directory opens.
/// Files and directories draw from one sequence: the
/// `lv2_fs_answers` capture reads 3, 4 and 5 for three file opens,
/// then 6 for a fourth and 7 for a directory. The three opens follow
/// three create-and-close pairs, so the console hands a closed fd's
/// number out again.
///
/// File fds are small ints in `[3, 255]` on real PS3 (see
/// [`LV2_FS_OBJECT_ID_LAST`]). Titles encode the fd into narrow struct
/// fields and load it with `lbz`/`lhz`/`lwz` semantics that truncate
/// high bits; returning fds in the billions corrupts the fd in the
/// title's internal table.
pub const LV2_FS_OBJECT_ID_BASE: u32 = 3;

/// The last fd the kernel hands out. One more open while every fd from
/// [`LV2_FS_OBJECT_ID_BASE`] to this one is held is `CELL_EMFILE`.
// The console capture of tests/micro/lv2_error_codes_fs_more opens one
// file under /dev_hdd0/tmp 253 times, on fds 3 to 255, and the 254th
// open answers EMFILE.
pub const LV2_FS_OBJECT_ID_LAST: u32 = 255;

/// The roots a game process finds mounted. An absolute path under any
/// other root is `CELL_ENOTMOUNTED` from `sys_fs_open` and
/// `sys_fs_opendir`; a missing name under one of these is `CELL_ENOENT`.
// The console capture of tests/micro/lv2_fs_unmounted_paths: a missing
// name under /dev_hdd0, /app_home and /host_root answers ENOENT, as does
// a path with no leading slash. /dev_usb007 (nothing plugged in),
// /dev_hdd1, /dev_cgov and /cgov_nodev answer ENOTMOUNTED. The
// tests/micro/lv2_error_codes_fs_more capture answers ENOTMOUNTED for
// /dev_bdvd with no disc inserted. The flash mounts follow the firmware
// flash layout (format::dev_flash); the lv2_fs_blksize and
// lv2_fs_constants captures read under /dev_flash, and no capture reads
// under /dev_flash2 or /dev_flash3. CellGov models no
// USB device, so /dev_usb000 is not listed although the reference
// console has one plugged in.
pub const GAME_PROCESS_MOUNTED_ROOTS: [&str; 6] = [
    "/dev_flash",
    "/dev_flash2",
    "/dev_flash3",
    "/dev_hdd0",
    "/app_home",
    "/host_root",
];

// -- sys_fs_open flag bits --
//
// The console capture of tests/micro/lv2_fs_constants, from a game
// process on a file under /dev_hdd0, shows each bit's effect: a write on
// an O_RDONLY fd and a read on an O_WRONLY fd answer EBADF; O_RDWR and
// O_ACCMODE itself open; O_CREAT | O_EXCL on an existing file answers
// EEXIST; O_APPEND leaves a 4-byte file 5 bytes long with the position
// at 5 after a 1-byte write; O_TRUNC leaves it 0 bytes long; O_CREAT
// makes a new file with the mode it names. With O_RDONLY, every other
// bit from 0x4 to 0x20000 opens except 0x1000, O_MSELF.

/// `CELL_FS_O_RDONLY`: open for reading.
pub const CELL_FS_O_RDONLY: u32 = 0o0;
/// `CELL_FS_O_WRONLY`: open for writing.
pub const CELL_FS_O_WRONLY: u32 = 0o1;
/// `CELL_FS_O_RDWR`: open for reading and writing.
pub const CELL_FS_O_RDWR: u32 = 0o2;
/// Mask for the access-mode subfield (`RDONLY | WRONLY | RDWR`). The
/// value itself opens.
pub const CELL_FS_O_ACCMODE: u32 = 0o3;
/// `CELL_FS_O_CREAT`: create file if it does not exist.
pub const CELL_FS_O_CREAT: u32 = 0o100;
/// `CELL_FS_O_EXCL`: combined with O_CREAT, fail with `CELL_EEXIST` if
/// the file exists.
pub const CELL_FS_O_EXCL: u32 = 0o200;
/// `CELL_FS_O_TRUNC`: truncate file on open.
pub const CELL_FS_O_TRUNC: u32 = 0o1000;
/// `CELL_FS_O_APPEND`: append on every write.
pub const CELL_FS_O_APPEND: u32 = 0o2000;
/// `CELL_FS_O_MSELF`: open the file as an MSELF archive. A file that is
/// not one is `CELL_ENOTMSELF`.
pub const CELL_FS_O_MSELF: u32 = 0o10000;

// -- Path length cap --

/// The longest a path may run below its first component, the device
/// it names (`/dev_hdd0`, `/dev_flash`), not counting the NUL. One byte
/// more is `CELL_ENAMETOOLONG`.
// The console capture of tests/micro/lv2_fs_path_limit_mount takes a
// path of 1033 bytes under /dev_hdd0 and 1034 under /dev_flash and
// refuses one byte more under each.
pub const CELL_FS_MAX_PATH_LENGTH: usize = 1024;

// -- CellFsStat wire format --

/// Wire size of the `CellFsStat` record `sys_fs_stat` and
/// `sys_fs_fstat` write: `{ s32 mode; s32 uid; s32 gid; s64 atime;
/// s64 mtime; s64 ctime; u64 size; u64 blksize }`, packed with no
/// padding. The `lv2_fs_answers` capture reads the size at offset 36
/// and the block size at offset 44, and the kernel leaves bytes 52 on
/// untouched; in the `lv2_fs_constants` capture `sys_fs_stat` and
/// `sys_fs_fstat` each write exactly 52 bytes over a filled buffer.
pub const CELL_FS_STAT_SIZE: u64 = 52;

/// Offset of `CellFsStat::size` in the packed record.
pub const CELL_FS_STAT_SIZE_OFFSET: usize = 36;

/// Offset of `CellFsStat::blksize` in the packed record.
pub const CELL_FS_STAT_BLKSIZE_OFFSET: usize = 44;

/// `CellFsStat::blksize` for a file or directory, as the
/// `lv2_fs_answers` capture reads it under `/dev_hdd0` and the
/// `lv2_fs_blksize` capture under `/dev_hdd0/game`, `/dev_flash` and
/// `/dev_usb000`. `/app_home` is unprobed.
pub const CELL_FS_BLOCK_SIZE: u64 = 512;

/// `CellFsStat::blksize` for a file or directory on the game disc
/// (`/dev_bdvd`), as the `lv2_fs_blksize` capture reads it.
pub const CELL_FS_DISC_BLOCK_SIZE: u64 = 2048;

/// The mount point of the game disc.
pub const CELL_FS_DISC_ROOT: &str = "/dev_bdvd";

// -- mode bits (subset that CellGov currently emits) --
//
// The console captures read 0x8180 for a file made with mode 0o600
// (lv2_fs_answers, lv2_fs_constants), 0x41C0 for a directory made with
// 0o700, and 0x81A4 for a file under /dev_flash: S_IFREG, S_IFDIR and
// the owner, group and other read bits, and owner-execute. No capture
// shows group- or other-execute.

/// `S_IFREG`: regular-file mode bit.
pub const CELL_FS_S_IFREG: u32 = 0x8000;
/// `S_IFDIR`: directory mode bit. The `lv2_fs_answers` capture reads
/// `0x41C0` for a directory made with mode `0o700`.
pub const CELL_FS_S_IFDIR: u32 = 0x4000;
/// `S_IRUSR`: owner-read.
pub const CELL_FS_S_IRUSR: u32 = 0x100;
/// `S_IRGRP`: group-read.
pub const CELL_FS_S_IRGRP: u32 = 0x020;
/// `S_IROTH`: other-read.
pub const CELL_FS_S_IROTH: u32 = 0x004;
/// `S_IXUSR`: owner-execute (search, on a directory).
pub const CELL_FS_S_IXUSR: u32 = 0x040;
/// `S_IXGRP`: group-execute.
pub const CELL_FS_S_IXGRP: u32 = 0x008;
/// `S_IXOTH`: other-execute.
pub const CELL_FS_S_IXOTH: u32 = 0x001;

// -- CellFsDirent wire format --

/// Wire size of `CellFsDirent`. The struct is
/// `{ u8 d_type; u8 d_namlen; char d_name[256] }` with the name
/// stored as a fixed 256-byte buffer (NUL-padded).
// The console capture of tests/micro/lv2_fs_constants: each readdir
// writes 258 bytes over a filled buffer and reports nread 258, and the
// call at the end of the directory writes 258 bytes too and reports 0.
pub const CELL_FS_DIRENT_SIZE: u64 = 258;

/// The longest name a path may carry, excluding the trailing NUL, and
/// so the longest `CellFsDirent::d_name` holds. A path naming a longer
/// one is `CELL_ENAMETOOLONG`.
// The console capture of tests/micro/lv2_fs_constants: sys_fs_stat of
// a 255-byte name answers ENOENT, of a 256-byte one ENAMETOOLONG.
pub const CELL_FS_MAX_FS_FILE_NAME_LENGTH: u8 = 255;

/// `d_type` for an unknown filesystem entry. No capture reads it.
pub const CELL_FS_TYPE_UNKNOWN: u8 = 0;
/// `d_type` for a directory, as the `lv2_fs_answers` and
/// `lv2_fs_constants` captures read it for `.` and `..`.
pub const CELL_FS_TYPE_DIRECTORY: u8 = 1;
/// `d_type` for a regular file, as the same captures read it.
pub const CELL_FS_TYPE_REGULAR: u8 = 2;
/// `d_type` for a symlink. No capture reads it: no probe has a symlink
/// to list.
pub const CELL_FS_TYPE_SYMLINK: u8 = 3;
