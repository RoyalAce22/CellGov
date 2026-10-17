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
/// File fds are small ints in `[3, 255)` on real PS3. Titles encode
/// the fd into narrow struct fields and load it with `lbz`/`lhz`/
/// `lwz` semantics that truncate high bits; returning fds in the
/// billions corrupts the fd in the title's internal table.
pub const LV2_FS_OBJECT_ID_BASE: u32 = 3;

// -- sys_fs_open flag bits --

/// `CELL_FS_O_RDONLY`: open for reading.
pub const CELL_FS_O_RDONLY: u32 = 0o0;
/// `CELL_FS_O_WRONLY`: open for writing.
pub const CELL_FS_O_WRONLY: u32 = 0o1;
/// `CELL_FS_O_RDWR`: open for reading and writing.
pub const CELL_FS_O_RDWR: u32 = 0o2;
/// Mask for the access-mode subfield (`RDONLY | WRONLY | RDWR`).
pub const CELL_FS_O_ACCMODE: u32 = 0o3;
/// `CELL_FS_O_CREAT`: create file if it does not exist.
pub const CELL_FS_O_CREAT: u32 = 0o100;
/// `CELL_FS_O_EXCL`: combined with O_CREAT, fail if file exists.
pub const CELL_FS_O_EXCL: u32 = 0o200;
/// `CELL_FS_O_TRUNC`: truncate file on open.
pub const CELL_FS_O_TRUNC: u32 = 0o1000;
/// `CELL_FS_O_APPEND`: append on every write.
pub const CELL_FS_O_APPEND: u32 = 0o2000;

// -- Path length cap --

/// `CELL_FS_MAX_PATH_LENGTH`. Counts the terminator: max content
/// is `CELL_FS_MAX_PATH_LENGTH - 1` and the NUL must appear at
/// index `<= 1023`.
pub const CELL_FS_MAX_PATH_LENGTH: usize = 1024;

// -- CellFsStat wire format --

/// Wire size of the `CellFsStat` record `sys_fs_stat` and
/// `sys_fs_fstat` write: `{ s32 mode; s32 uid; s32 gid; s64 atime;
/// s64 mtime; s64 ctime; u64 size; u64 blksize }`, packed with no
/// padding. The `lv2_fs_answers` capture reads the size at offset 36
/// and the block size at offset 44, and the kernel leaves bytes 52 on
/// untouched.
pub const CELL_FS_STAT_SIZE: u64 = 52;

/// Offset of `CellFsStat::size` in the packed record.
pub const CELL_FS_STAT_SIZE_OFFSET: usize = 36;

/// Offset of `CellFsStat::blksize` in the packed record.
pub const CELL_FS_STAT_BLKSIZE_OFFSET: usize = 44;

/// `CellFsStat::blksize` for a file or directory under `/dev_hdd0`,
/// as the `lv2_fs_answers` capture reads it. Other devices are
/// unprobed.
pub const CELL_FS_BLOCK_SIZE: u64 = 512;

// -- mode bits (subset that CellGov currently emits) --

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
pub const CELL_FS_DIRENT_SIZE: u64 = 258;

/// Maximum filename length the kernel will write into
/// `CellFsDirent::d_name`, excluding the trailing NUL. Names
/// longer than this are truncated; `d_namlen` clamps to this
/// value so guests reading the prefix get a well-formed string.
pub const CELL_FS_MAX_FS_FILE_NAME_LENGTH: u8 = 255;

/// `d_type` for an unknown filesystem entry.
pub const CELL_FS_TYPE_UNKNOWN: u8 = 0;
/// `d_type` for a directory.
pub const CELL_FS_TYPE_DIRECTORY: u8 = 1;
/// `d_type` for a regular file.
pub const CELL_FS_TYPE_REGULAR: u8 = 2;
/// `d_type` for a symlink.
pub const CELL_FS_TYPE_SYMLINK: u8 = 3;
