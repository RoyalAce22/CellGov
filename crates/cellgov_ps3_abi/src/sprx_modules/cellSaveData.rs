//! cellSaveData PS3 ABI: error codes, callback return codes, struct
//! layouts, and the misc enum constants the autoload / autosave
//! callback contract reads and writes.
//!
//! Mirrors the layout of RPCS3's
//! `rpcs3/Emu/Cell/Modules/cellSaveData.h`. Behaviour (the
//! callback dispatch on a worker thread, the directory enumeration,
//! the file-iteration loop) lives in `cellgov_core::hle::cellSaveData`
//! when implemented; this module is data only.
//!
//! The autoload contract (`cellSaveDataAutoLoad` /
//! `cellSaveDataAutoLoad2`) takes two title-supplied callbacks:
//! `funcStat(cbResult, statGet, statSet)` runs first, then
//! `funcFile(cbResult, fileGet, fileSet)` runs in a loop. A title
//! that called autoload and never saw the callbacks fire reads back
//! whatever was in `cbResult` / `statSet` / `fileSet` at call time;
//! the canonical failure mode on titles that depend on this surface
//! is a NULL-bcctr in post-AutoLoad code where a vtable slot the
//! title's funcStat was supposed to populate stays zero.

/// `CellSaveDataError` band (`0x8002_b40x`).
pub mod error {
    /// CBResult signaled an error to the title.
    pub const CBRESULT: u32 = 0x8002_b401;
    /// Filesystem access error during directory enumeration or fileop.
    pub const ACCESS_ERROR: u32 = 0x8002_b402;
    /// Internal error.
    pub const INTERNAL: u32 = 0x8002_b403;
    /// Invalid argument (null pointer, bad enum, bad combination).
    pub const PARAM: u32 = 0x8002_b404;
    /// Out of HDD space for the requested write.
    pub const NOSPACE: u32 = 0x8002_b405;
    /// Save-data directory is corrupt.
    pub const BROKEN: u32 = 0x8002_b406;
    /// Generic failure.
    pub const FAILURE: u32 = 0x8002_b407;
    /// Save-data subsystem is busy with another caller.
    pub const BUSY: u32 = 0x8002_b408;
    /// No user logged in.
    pub const NOUSER: u32 = 0x8002_b409;
    /// Per-file or per-directory size exceeded the configured cap.
    pub const SIZEOVER: u32 = 0x8002_b40a;
    /// No save data found for the autoload directory.
    pub const NODATA: u32 = 0x8002_b40b;
    /// Operation not supported (e.g. dialog variant on a no-UI build).
    pub const NOTSUPPORTED: u32 = 0x8002_b40c;

    /// Sign-extend a `cellSaveData` error code to the 64-bit shape
    /// PS3's PPC64 calling convention puts in r3 on syscall return.
    ///
    /// Real PS3 / RPCS3 returns `int32_t` from the cellSaveData
    /// entrypoints; PPC64 sign-extends a 32-bit return into the full
    /// 64-bit GPR. Every error code in this band has the high bit
    /// set (`0x8002_b4xx`), so the 64-bit value is `0xFFFF_FFFF_8002_b4xx`,
    /// not `0x0000_0000_8002_b4xx`. Guests doing a doubleword signed
    /// compare (`cmpdi r3, 0`) read negative under sign-extension and
    /// positive under zero-extension; only the former matches real
    /// hardware.
    ///
    /// Use this helper rather than `code as u64` at every
    /// cellSaveData-error syscall-return site. The helper is `const`
    /// so it works in const contexts and at compile-time test
    /// fixtures.
    #[inline]
    pub const fn as_r3(code: u32) -> u64 {
        code as i32 as i64 as u64
    }
}

/// `CellSaveDataCBResult.result` values written by title callbacks
/// to direct savedata_op's control flow.
///
/// Written by every `funcStat` / `funcFile` / `funcDone` / `funcList`
/// / `funcFixed` callback before returning.
pub mod cb_result {
    /// Continue: drop into the next callback in sequence.
    pub const OK_NEXT: i32 = 0;
    /// Stop here: return CELL_OK to the title without further callbacks.
    pub const OK_LAST: i32 = 1;
    /// Stop without rendering the system confirmation dialog. Only
    /// legal for `funcFile` and `funcDone`; `funcStat` returning this
    /// is rejected by RPCS3's savedata_op.
    pub const OK_LAST_NOCONFIRM: i32 = 2;
    /// Title-side error: insufficient HDD space.
    pub const ERR_NOSPACE: i32 = -1;
    /// Title-side error: generic failure.
    pub const ERR_FAILURE: i32 = -2;
    /// Title-side error: save data is broken.
    pub const ERR_BROKEN: i32 = -3;
    /// Title-side error: no save data exists at the queried directory.
    pub const ERR_NODATA: i32 = -4;
    /// Title-side error: invalid argument.
    pub const ERR_INVALID: i32 = -5;
}

/// String-buffer sizes inside the savedata structs.
pub mod size {
    /// Length of the save-data directory name buffer (without NUL).
    pub const DIRNAME: u32 = 32;
    /// Length of the file name buffer (without NUL).
    pub const FILENAME: u32 = 13;
    /// Length of the secure-file id field.
    pub const SECUREFILEID: u32 = 16;
    /// Length of the directory-prefix buffer for list operations.
    pub const PREFIX: u32 = 256;
    /// Title field width in `CellSaveDataSystemFileParam`.
    pub const SYSP_TITLE: u32 = 128;
    /// Subtitle field width.
    pub const SYSP_SUBTITLE: u32 = 128;
    /// Detail field width.
    pub const SYSP_DETAIL: u32 = 1024;
    /// Listparam field width (8 bytes).
    pub const SYSP_LPARAM: u32 = 8;
}

/// `CellSaveDataIsNewData` values written into `statGet->isNewData`.
pub mod is_new_data {
    /// Save data exists at the queried directory; populated `dir` /
    /// `getParam` / `fileList` are valid.
    pub const NO: u32 = 0;
    /// No save data found; the `dir` / `getParam` fields are zeroed
    /// and the title is expected to fill in `statSet->setParam` to
    /// describe the fresh directory it wants created.
    pub const YES: u32 = 1;
}

/// `CellSaveDataFileOperation` values the title sets in
/// `fileSet->fileOperation`.
pub mod file_op {
    /// Read `fileSize` bytes at `fileOffset` into `fileBuf`.
    pub const READ: u32 = 0;
    /// Write `fileSize` bytes from `fileBuf` to `fileOffset`,
    /// truncating the rest of the file.
    pub const WRITE: u32 = 1;
    /// Delete the named file.
    pub const DELETE: u32 = 2;
    /// Write at offset without truncating beyond the written range.
    pub const WRITE_NOTRUNC: u32 = 3;
}

/// `CellSaveDataFileType` values written into `fileSet->fileType`.
pub mod file_type {
    /// Title-defined data file with secure-file-id checksum.
    pub const SECUREFILE: u32 = 0;
    /// Plain title data file (no checksum).
    pub const NORMALFILE: u32 = 1;
    /// PSP/PS3 save-data icon (256 KB cap, PNG).
    pub const CONTENT_ICON0: u32 = 2;
    /// Animated icon (1 MB cap, MJPG).
    pub const CONTENT_ICON1: u32 = 3;
    /// Background image (16 MB cap, PNG / JPG).
    pub const CONTENT_PIC1: u32 = 4;
    /// Save-data sound (1 MB cap, AT3).
    pub const CONTENT_SND0: u32 = 5;
}

/// `CellSaveDataSystemFileParam.attribute` flags.
pub mod attr {
    /// Normal save: another instance of the same dirName overwrites.
    pub const NORMAL: u32 = 0;
    /// No-duplicate: re-saving over the same dirName fails with
    /// CELL_SAVEDATA_ERROR_FAILURE.
    pub const NODUPLICATE: u32 = 1;
}

// Field offsets and sizes for the structs the AutoLoad / AutoSave
// callbacks read and write. Pointer fields (`vm::bptr<T>`) are 4-byte
// big-endian guest pointers, since LV2 is a 32-bit user-space ABI.

/// `CellSaveDataCBResult` -- the per-callback result block the title
/// writes before returning. Always passed by pointer to every
/// callback in the savedata family.
pub mod cb_result_layout {
    /// `result` (s32) -- one of the `cb_result::*` discriminants.
    pub const OFF_RESULT: u32 = 0x00;
    /// `progressBarInc` (u32) -- progress-bar percent advance for
    /// long autosave/autoload operations.
    pub const OFF_PROGRESS_BAR_INC: u32 = 0x04;
    /// `errNeedSizeKB` (s32) -- KiB the title wants the system to
    /// free before retrying. Negative on error paths.
    pub const OFF_ERR_NEED_SIZE_KB: u32 = 0x08;
    /// `invalidMsg` (vm::bptr<char>, 4 bytes BE).
    pub const OFF_INVALID_MSG: u32 = 0x0C;
    /// `userdata` (vm::bptr<void>, 4 bytes BE).
    pub const OFF_USERDATA: u32 = 0x10;
    /// Total size: 20 bytes.
    pub const SIZE: u32 = 0x14;
}

/// `CellSaveDataDirStat` -- the parsed savedata directory header.
/// Embedded inside `CellSaveDataStatGet` at offset 8.
pub mod dir_stat_layout {
    /// `atime` (s64).
    pub const OFF_ATIME: u32 = 0x00;
    /// `mtime` (s64).
    pub const OFF_MTIME: u32 = 0x08;
    /// `ctime` (s64).
    pub const OFF_CTIME: u32 = 0x10;
    /// `dirName` (char[32]).
    pub const OFF_DIR_NAME: u32 = 0x18;
    /// Total struct size.
    pub const SIZE: u32 = 0x38;
}

/// `CellSaveDataFileStat` -- one entry in `statGet->fileList`.
pub mod file_stat_layout {
    /// `fileType` (u32) -- one of the `file_type::*` discriminants.
    pub const OFF_FILE_TYPE: u32 = 0x00;
    /// `size` (u64). 4 bytes of reserved padding follow `fileType`
    /// before the size field starts.
    pub const OFF_SIZE_BYTES: u32 = 0x08;
    /// `atime` (s64).
    pub const OFF_ATIME: u32 = 0x10;
    /// `mtime` (s64).
    pub const OFF_MTIME: u32 = 0x18;
    /// `ctime` (s64).
    pub const OFF_CTIME: u32 = 0x20;
    /// `fileName` (char[13]) followed by 3 reserved bytes.
    pub const OFF_FILE_NAME: u32 = 0x28;
    /// Total struct size.
    pub const SIZE: u32 = 0x38;
}

/// `CellSaveDataSystemFileParam` -- the per-save metadata block.
/// Embedded inside `CellSaveDataStatGet` at offset 0x40 and
/// pointed-to by `statSet->setParam`.
pub mod system_file_param_layout {
    /// `title` (char[128]).
    pub const OFF_TITLE: u32 = 0x000;
    /// `subTitle` (char[128]).
    pub const OFF_SUB_TITLE: u32 = 0x080;
    /// `detail` (char[1024]).
    pub const OFF_DETAIL: u32 = 0x100;
    /// `attribute` (u32) -- one of the `attr::*` flags.
    pub const OFF_ATTRIBUTE: u32 = 0x500;
    /// `parental_level` (u32). Firmware 3.70+ relabels the field
    /// `reserved2[4]`; the layout offset is identical.
    pub const OFF_PARENTAL_LEVEL: u32 = 0x504;
    /// `listParam` (char[8]).
    pub const OFF_LIST_PARAM: u32 = 0x508;
    /// Total struct size; 256 reserved bytes follow `listParam`.
    pub const SIZE: u32 = 0x610;
}

/// `CellSaveDataAutoIndicator` -- the optional progress overlay
/// configuration the title can attach via `statSet->indicator`.
pub mod auto_indicator_layout {
    /// `dispPosition` (u32).
    pub const OFF_DISP_POSITION: u32 = 0x00;
    /// `dispMode` (u32).
    pub const OFF_DISP_MODE: u32 = 0x04;
    /// `dispMsg` (vm::bptr<char>).
    pub const OFF_DISP_MSG: u32 = 0x08;
    /// `picBufSize` (u32).
    pub const OFF_PIC_BUF_SIZE: u32 = 0x0C;
    /// `picBuf` (vm::bptr<void>).
    pub const OFF_PIC_BUF: u32 = 0x10;
    /// Total struct size; the trailing 4 bytes are a reserved
    /// `vm::bptr<void>`.
    pub const SIZE: u32 = 0x18;
}

/// `CellSaveDataStatGet` -- populated by the system before
/// `funcStat` fires, read by the title callback.
pub mod stat_get_layout {
    /// `hddFreeSizeKB` (s32).
    pub const OFF_HDD_FREE_SIZE_KB: u32 = 0x000;
    /// `isNewData` (u32) -- one of the `is_new_data::*` discriminants.
    pub const OFF_IS_NEW_DATA: u32 = 0x004;
    /// Embedded `CellSaveDataDirStat` (size 0x38).
    pub const OFF_DIR: u32 = 0x008;
    /// Embedded `CellSaveDataSystemFileParam` (size 0x610).
    pub const OFF_GET_PARAM: u32 = 0x040;
    /// `bind` (u32) -- bitmask of `CELL_SAVEDATA_BINDSTAT_*`.
    pub const OFF_BIND: u32 = 0x650;
    /// `sizeKB` (s32) -- save-data total disk usage.
    pub const OFF_SIZE_KB: u32 = 0x654;
    /// `sysSizeKB` (s32) -- system-side overhead, fixed at 35 KiB.
    pub const OFF_SYS_SIZE_KB: u32 = 0x658;
    /// `fileNum` (u32) -- count of files actually present.
    pub const OFF_FILE_NUM: u32 = 0x65C;
    /// `fileListNum` (u32) -- count of entries materialised in
    /// `fileList`, capped by `setBuf->fileListMax`.
    pub const OFF_FILE_LIST_NUM: u32 = 0x660;
    /// `fileList` (vm::bptr<CellSaveDataFileStat>).
    pub const OFF_FILE_LIST: u32 = 0x664;
    /// Total struct size; 64 reserved bytes follow `fileList`.
    pub const SIZE: u32 = 0x6A8;
}

/// `CellSaveDataStatSet` -- populated by the title callback before
/// returning, read by the system to drive the file-iteration loop.
pub mod stat_set_layout {
    /// `setParam` (vm::bptr<CellSaveDataSystemFileParam>).
    pub const OFF_SET_PARAM: u32 = 0x00;
    /// `reCreateMode` (u32) -- 0 keep, 1 keep-no-broken, 2 wipe,
    /// 3 wipe-and-reset-owner.
    pub const OFF_RECREATE_MODE: u32 = 0x04;
    /// `indicator` (vm::bptr<CellSaveDataAutoIndicator>).
    pub const OFF_INDICATOR: u32 = 0x08;
    /// Total struct size.
    pub const SIZE: u32 = 0x0C;
}

/// `CellSaveDataFileGet` -- per-iteration file metadata the system
/// hands to `funcFile`.
pub mod file_get_layout {
    /// `excSize` (u32) -- bytes consumed by the previous fileop.
    pub const OFF_EXC_SIZE: u32 = 0x00;
    /// Total struct size; 64 reserved bytes follow `excSize`.
    pub const SIZE: u32 = 0x44;
}

/// `CellSaveDataFileSet` -- per-iteration file request the title
/// callback writes before returning.
pub mod file_set_layout {
    /// `fileOperation` (u32) -- one of the `file_op::*` discriminants.
    pub const OFF_FILE_OPERATION: u32 = 0x00;
    /// `reserved` (vm::bptr<void>).
    pub const OFF_RESERVED: u32 = 0x04;
    /// `fileType` (u32) -- one of the `file_type::*` discriminants.
    pub const OFF_FILE_TYPE: u32 = 0x08;
    /// `secureFileId` (u128, align 1) -- 16 bytes packed.
    pub const OFF_SECURE_FILE_ID: u32 = 0x0C;
    /// `fileName` (vm::bptr<char>).
    pub const OFF_FILE_NAME: u32 = 0x1C;
    /// `fileOffset` (u32).
    pub const OFF_FILE_OFFSET: u32 = 0x20;
    /// `fileSize` (u32) -- bytes the title wants read or written.
    pub const OFF_FILE_SIZE: u32 = 0x24;
    /// `fileBufSize` (u32) -- title-side buffer capacity.
    pub const OFF_FILE_BUF_SIZE: u32 = 0x28;
    /// `fileBuf` (vm::bptr<void>).
    pub const OFF_FILE_BUF: u32 = 0x2C;
    /// Total struct size.
    pub const SIZE: u32 = 0x30;
}

/// `CellSaveDataSetBuf` -- the title's allocator-side capacity hints
/// for the file / directory enumeration buffers, passed by pointer
/// in autoload arg position 4.
pub mod set_buf_layout {
    /// `dirListMax` (u32).
    pub const OFF_DIR_LIST_MAX: u32 = 0x00;
    /// `fileListMax` (u32).
    pub const OFF_FILE_LIST_MAX: u32 = 0x04;
    /// `bufSize` (u32). 24 bytes of reserved padding sit between
    /// `fileListMax` and `bufSize`.
    pub const OFF_BUF_SIZE: u32 = 0x20;
    /// `buf` (vm::bptr<void>).
    pub const OFF_BUF: u32 = 0x24;
    /// Total struct size.
    pub const SIZE: u32 = 0x28;
}

// Compile-time offset assertions. Catch a layout drift -- field
// reordered, padding miscounted, struct grown -- at build time so
// any silent guest-memory corruption (writing at the wrong offset,
// reading past a region limit) cannot ship.
const _: () = {
    // Sign-extension contract for r3-bound error codes. The PPC64
    // calling convention sign-extends `int32_t` returns into the
    // 64-bit r3, and every CellSaveDataError has the high bit set;
    // pin the helper so a future drift to zero-extension trips
    // here.
    assert!(error::as_r3(error::NODATA) == 0xFFFF_FFFF_8002_B40B);
    assert!(error::as_r3(error::PARAM) == 0xFFFF_FFFF_8002_B404);
    // CELL_OK case: zero stays zero under sign-extension.
    assert!(error::as_r3(0) == 0);

    // Self-consistency: every named field offset stays inside its
    // struct.
    assert!(cb_result_layout::OFF_USERDATA + 4 == cb_result_layout::SIZE);
    assert!(dir_stat_layout::OFF_DIR_NAME + size::DIRNAME == dir_stat_layout::SIZE);
    // file_stat: fileType (4) + reserved1 (4) + size (8) + atime/mtime/ctime (24) +
    // fileName (13) + reserved2 (3) = 56. The named offsets pin the gaps.
    assert!(file_stat_layout::OFF_FILE_TYPE + 8 == file_stat_layout::OFF_SIZE_BYTES);
    assert!(file_stat_layout::OFF_SIZE_BYTES + 8 == file_stat_layout::OFF_ATIME);
    assert!(file_stat_layout::OFF_ATIME + 8 == file_stat_layout::OFF_MTIME);
    assert!(file_stat_layout::OFF_MTIME + 8 == file_stat_layout::OFF_CTIME);
    assert!(file_stat_layout::OFF_CTIME + 8 == file_stat_layout::OFF_FILE_NAME);
    // Trailing 16 bytes = fileName (13) + reserved2 (3).
    assert!(file_stat_layout::OFF_FILE_NAME + 16 == file_stat_layout::SIZE);

    // SystemFileParam tile lengths against the size constants, then
    // total against the struct size.
    assert!(
        system_file_param_layout::OFF_TITLE + size::SYSP_TITLE
            == system_file_param_layout::OFF_SUB_TITLE
    );
    assert!(
        system_file_param_layout::OFF_SUB_TITLE + size::SYSP_SUBTITLE
            == system_file_param_layout::OFF_DETAIL
    );
    assert!(
        system_file_param_layout::OFF_DETAIL + size::SYSP_DETAIL
            == system_file_param_layout::OFF_ATTRIBUTE
    );
    assert!(
        system_file_param_layout::OFF_ATTRIBUTE + 4 == system_file_param_layout::OFF_PARENTAL_LEVEL
    );
    assert!(
        system_file_param_layout::OFF_PARENTAL_LEVEL + 4
            == system_file_param_layout::OFF_LIST_PARAM
    );
    // listParam (8) + reserved[256] = 264 trailing bytes.
    assert!(
        system_file_param_layout::OFF_LIST_PARAM + size::SYSP_LPARAM + 256
            == system_file_param_layout::SIZE
    );

    // StatGet: dir embed at 0x08 + 0x38 = 0x40, then SystemFileParam
    // embed at 0x40 + 0x610 = 0x650, then the trailing bind / size
    // / count fields, fileList pointer, and 64 reserved bytes ending
    // at 0x6A8.
    assert!(stat_get_layout::OFF_DIR + dir_stat_layout::SIZE == stat_get_layout::OFF_GET_PARAM);
    assert!(
        stat_get_layout::OFF_GET_PARAM + system_file_param_layout::SIZE
            == stat_get_layout::OFF_BIND
    );
    assert!(stat_get_layout::OFF_BIND + 4 == stat_get_layout::OFF_SIZE_KB);
    assert!(stat_get_layout::OFF_SIZE_KB + 4 == stat_get_layout::OFF_SYS_SIZE_KB);
    assert!(stat_get_layout::OFF_SYS_SIZE_KB + 4 == stat_get_layout::OFF_FILE_NUM);
    assert!(stat_get_layout::OFF_FILE_NUM + 4 == stat_get_layout::OFF_FILE_LIST_NUM);
    assert!(stat_get_layout::OFF_FILE_LIST_NUM + 4 == stat_get_layout::OFF_FILE_LIST);
    // fileList (4) + reserved[64] = 68 trailing bytes.
    assert!(stat_get_layout::OFF_FILE_LIST + 4 + 64 == stat_get_layout::SIZE);

    // StatSet is dense: setParam (4) + reCreateMode (4) + indicator (4) = 12.
    assert!(stat_set_layout::OFF_RECREATE_MODE + 4 == stat_set_layout::OFF_INDICATOR);
    assert!(stat_set_layout::OFF_INDICATOR + 4 == stat_set_layout::SIZE);

    // FileGet: excSize (4) + reserved[64] = 68.
    assert!(file_get_layout::OFF_EXC_SIZE + 4 + 64 == file_get_layout::SIZE);

    // FileSet: secureFileId is be_t<u128, 1> -- align 1 lets it sit
    // immediately after fileType at 0x0C. fileName follows at 0x1C.
    assert!(file_set_layout::OFF_FILE_TYPE + 4 == file_set_layout::OFF_SECURE_FILE_ID);
    assert!(
        file_set_layout::OFF_SECURE_FILE_ID + size::SECUREFILEID == file_set_layout::OFF_FILE_NAME
    );
    assert!(file_set_layout::OFF_FILE_NAME + 4 == file_set_layout::OFF_FILE_OFFSET);
    assert!(file_set_layout::OFF_FILE_OFFSET + 4 == file_set_layout::OFF_FILE_SIZE);
    assert!(file_set_layout::OFF_FILE_SIZE + 4 == file_set_layout::OFF_FILE_BUF_SIZE);
    assert!(file_set_layout::OFF_FILE_BUF_SIZE + 4 == file_set_layout::OFF_FILE_BUF);
    assert!(file_set_layout::OFF_FILE_BUF + 4 == file_set_layout::SIZE);

    // SetBuf: dirListMax (4) + fileListMax (4) + reserved[6] (24) = 32 bytes
    // before bufSize, then bufSize (4) + buf (4) = 40.
    assert!(set_buf_layout::OFF_FILE_LIST_MAX + 4 + 24 == set_buf_layout::OFF_BUF_SIZE);
    assert!(set_buf_layout::OFF_BUF_SIZE + 4 == set_buf_layout::OFF_BUF);
    assert!(set_buf_layout::OFF_BUF + 4 == set_buf_layout::SIZE);

    // AutoIndicator: 6 4-byte fields = 24 bytes.
    assert!(auto_indicator_layout::OFF_DISP_MODE + 4 == auto_indicator_layout::OFF_DISP_MSG);
    assert!(auto_indicator_layout::OFF_DISP_MSG + 4 == auto_indicator_layout::OFF_PIC_BUF_SIZE);
    assert!(auto_indicator_layout::OFF_PIC_BUF_SIZE + 4 == auto_indicator_layout::OFF_PIC_BUF);
    assert!(auto_indicator_layout::OFF_PIC_BUF + 4 + 4 == auto_indicator_layout::SIZE);
};
