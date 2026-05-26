//! PS3 `sys_fs` ABI constants: open flags, the `CellFsStat` wire
//! format, mode bits, and the maximum path length.
//!
//! Octal literals match the canonical PS3 `cell_fs.h` form (using
//! hex risks silent transcription errors on the order-of-magnitude
//! nibble: `O_CREAT = 0o100 = 0x40`, NOT `0x4`).

/// `lv2_fs_object::id_base` (per RPCS3's `sys_fs.h`): the starting
/// fd value the kernel hands out for file/dir opens.
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

/// Wire size of `CellFsStat`. The PS3 struct is
/// `{ s32 mode; s32 uid; s32 gid; <pad4>; s64 atime; s64 mtime;
/// s64 ctime; u64 size; u64 blksize }`; 8-byte alignment of the
/// 64-bit fields forces 4 bytes of padding between gid and atime.
pub const CELL_FS_STAT_SIZE: u64 = 56;

/// `CellFsStat::blksize` reported on PS3. 4096 is the IO block
/// size titles use to size their read buffers.
pub const CELL_FS_BLOCK_SIZE: u64 = 4096;

// -- mode bits (subset that CellGov currently emits) --

/// `S_IFREG`: regular-file mode bit.
pub const CELL_FS_S_IFREG: u32 = 0x8000;
/// `S_IRUSR`: owner-read.
pub const CELL_FS_S_IRUSR: u32 = 0x100;
/// `S_IRGRP`: group-read.
pub const CELL_FS_S_IRGRP: u32 = 0x020;
/// `S_IROTH`: other-read.
pub const CELL_FS_S_IROTH: u32 = 0x004;

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
