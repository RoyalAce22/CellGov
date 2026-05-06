//! cellSaveData PS3 ABI constants and struct layouts. Data only;
//! callback dispatch lives in `cellgov_core::hle::cellSaveData`.
//!
//! Layouts mirror `rpcs3/Emu/Cell/Modules/cellSaveData.h`. Pointer
//! fields (`vm::bptr<T>`) are 4-byte big-endian guest pointers.
//!
//! Autoload runs `funcStat(cbResult, statGet, statSet)` then
//! `funcFile(cbResult, fileGet, fileSet)` in a loop. If the system
//! never invokes the callbacks, the title reads back whatever was
//! in `cbResult` / `statSet` / `fileSet` at call time -- so any
//! caller of this surface must zero those buffers before the call.

/// `CellSaveDataError` band (`0x8002_b40x`).
pub mod error {
    /// kernel error: CBRESULT (callback result is malformed).
    pub const CBRESULT: u32 = 0x8002_b401;
    /// kernel error: ACCESS_ERROR.
    pub const ACCESS_ERROR: u32 = 0x8002_b402;
    /// kernel error: INTERNAL.
    pub const INTERNAL: u32 = 0x8002_b403;
    /// kernel error: PARAM.
    pub const PARAM: u32 = 0x8002_b404;
    /// kernel error: NOSPACE.
    pub const NOSPACE: u32 = 0x8002_b405;
    /// kernel error: BROKEN.
    pub const BROKEN: u32 = 0x8002_b406;
    /// kernel error: FAILURE.
    pub const FAILURE: u32 = 0x8002_b407;
    /// kernel error: BUSY.
    pub const BUSY: u32 = 0x8002_b408;
    /// kernel error: NOUSER.
    pub const NOUSER: u32 = 0x8002_b409;
    /// kernel error: SIZEOVER.
    pub const SIZEOVER: u32 = 0x8002_b40a;
    /// kernel error: NODATA.
    pub const NODATA: u32 = 0x8002_b40b;
    /// kernel error: NOTSUPPORTED.
    pub const NOTSUPPORTED: u32 = 0x8002_b40c;

    /// Sign-extend a 32-bit error code into the 64-bit r3 value the
    /// PPC64 ABI delivers from an `int32_t` return. Every code in
    /// this band has bit 31 set, so guests doing `cmpdi r3, 0` only
    /// read negative when sign-extended.
    #[inline]
    pub const fn as_r3(code: u32) -> u64 {
        code as i32 as i64 as u64
    }
}

/// `CellSaveDataCBResult.result` values written by title callbacks
/// to direct savedata_op control flow.
pub mod cb_result {
    /// Continue invoking callbacks for the next entry.
    pub const OK_NEXT: i32 = 0;
    /// Final entry; commit and finish savedata_op.
    pub const OK_LAST: i32 = 1;
    /// Only legal for `funcFile` and `funcDone`; `funcStat` returning
    /// this is rejected by savedata_op.
    pub const OK_LAST_NOCONFIRM: i32 = 2;
    /// Title aborted with insufficient-space error.
    pub const ERR_NOSPACE: i32 = -1;
    /// Title aborted with generic failure.
    pub const ERR_FAILURE: i32 = -2;
    /// Title aborted with broken-data error.
    pub const ERR_BROKEN: i32 = -3;
    /// Title aborted with no-data error.
    pub const ERR_NODATA: i32 = -4;
    /// Title aborted with invalid-parameter error.
    pub const ERR_INVALID: i32 = -5;
}

/// String-buffer sizes inside the savedata structs. Lengths exclude
/// the trailing NUL where applicable.
pub mod size {
    /// Buffer size in bytes for `dirName`.
    pub const DIRNAME: u32 = 32;
    /// Buffer size in bytes for `fileName`.
    pub const FILENAME: u32 = 13;
    /// Buffer size in bytes for `secureFileId`.
    pub const SECUREFILEID: u32 = 16;
    /// Buffer size in bytes for `prefix`.
    pub const PREFIX: u32 = 256;
    /// Buffer size in bytes for `SystemFileParam.title`.
    pub const SYSP_TITLE: u32 = 128;
    /// Buffer size in bytes for `SystemFileParam.subTitle`.
    pub const SYSP_SUBTITLE: u32 = 128;
    /// Buffer size in bytes for `SystemFileParam.detail`.
    pub const SYSP_DETAIL: u32 = 1024;
    /// Buffer size in bytes for `SystemFileParam.listParam`.
    pub const SYSP_LPARAM: u32 = 8;
}

/// `CellSaveDataIsNewData` values written into `statGet->isNewData`.
/// On `YES`, the `dir` and `getParam` embeds are zeroed and the
/// title must populate `statSet->setParam`.
pub mod is_new_data {
    /// Existing save data is being opened.
    pub const NO: u32 = 0;
    /// Save data does not yet exist; title must populate `setParam`.
    pub const YES: u32 = 1;
}

/// `CellSaveDataFileOperation` values written to
/// `fileSet->fileOperation`.
pub mod file_op {
    /// Read the requested file.
    pub const READ: u32 = 0;
    /// Write the requested file, truncating to the written length.
    pub const WRITE: u32 = 1;
    /// Delete the requested file.
    pub const DELETE: u32 = 2;
    /// Write at offset without truncating beyond the written range.
    pub const WRITE_NOTRUNC: u32 = 3;
}

/// `CellSaveDataFileType` values. Content types carry per-type size
/// caps enforced by the system: ICON0 256 KiB PNG, ICON1 1 MiB MJPG,
/// PIC1 16 MiB PNG/JPG, SND0 1 MiB AT3.
pub mod file_type {
    /// Secure file (encrypted with `secureFileId`).
    pub const SECUREFILE: u32 = 0;
    /// Plain (non-secure) file.
    pub const NORMALFILE: u32 = 1;
    /// `ICON0.PNG` thumbnail.
    pub const CONTENT_ICON0: u32 = 2;
    /// `ICON1.PAM` animated thumbnail.
    pub const CONTENT_ICON1: u32 = 3;
    /// `PIC1.PNG` background image.
    pub const CONTENT_PIC1: u32 = 4;
    /// `SND0.AT3` background sound.
    pub const CONTENT_SND0: u32 = 5;
}

/// `CellSaveDataSystemFileParam.attribute` flags. `NODUPLICATE`
/// makes a second save to the same dirName fail with
/// `error::FAILURE`.
pub mod attr {
    /// Default attribute; duplicates allowed.
    pub const NORMAL: u32 = 0;
    /// Reject a second save under the same `dirName`.
    pub const NODUPLICATE: u32 = 1;
}

/// `CellSaveDataCBResult` (20 bytes).
pub mod cb_result_layout {
    /// Byte offset of `result`.
    pub const OFF_RESULT: u32 = 0x00;
    /// Byte offset of `progressBarInc`.
    pub const OFF_PROGRESS_BAR_INC: u32 = 0x04;
    /// Byte offset of `errNeedSizeKB`.
    pub const OFF_ERR_NEED_SIZE_KB: u32 = 0x08;
    /// Byte offset of `invalidMsg`.
    pub const OFF_INVALID_MSG: u32 = 0x0C;
    /// Byte offset of `userdata`.
    pub const OFF_USERDATA: u32 = 0x10;
    /// Total struct size in bytes.
    pub const SIZE: u32 = 0x14;
}

/// `CellSaveDataDirStat` (0x38). Embedded in `CellSaveDataStatGet`
/// at 0x08.
pub mod dir_stat_layout {
    /// Byte offset of `atime`.
    pub const OFF_ATIME: u32 = 0x00;
    /// Byte offset of `mtime`.
    pub const OFF_MTIME: u32 = 0x08;
    /// Byte offset of `ctime`.
    pub const OFF_CTIME: u32 = 0x10;
    /// Byte offset of `dirName`.
    pub const OFF_DIR_NAME: u32 = 0x18;
    /// Total struct size in bytes.
    pub const SIZE: u32 = 0x38;
}

/// `CellSaveDataFileStat` (0x38). One entry in
/// `statGet->fileList`. Four bytes of padding sit between
/// `fileType` and `size`; three trailing bytes after `fileName`.
pub mod file_stat_layout {
    /// Byte offset of `fileType`.
    pub const OFF_FILE_TYPE: u32 = 0x00;
    /// Byte offset of `size` (in bytes).
    pub const OFF_SIZE_BYTES: u32 = 0x08;
    /// Byte offset of `atime`.
    pub const OFF_ATIME: u32 = 0x10;
    /// Byte offset of `mtime`.
    pub const OFF_MTIME: u32 = 0x18;
    /// Byte offset of `ctime`.
    pub const OFF_CTIME: u32 = 0x20;
    /// Byte offset of `fileName`.
    pub const OFF_FILE_NAME: u32 = 0x28;
    /// Total struct size in bytes.
    pub const SIZE: u32 = 0x38;
}

/// `CellSaveDataSystemFileParam` (0x610). Embedded in `StatGet` at
/// 0x40 and pointed-to by `statSet->setParam`. 256 reserved bytes
/// trail `listParam`. Firmware 3.70+ relabels `parental_level` as
/// `reserved2[4]` at the same offset.
pub mod system_file_param_layout {
    /// Byte offset of `title`.
    pub const OFF_TITLE: u32 = 0x000;
    /// Byte offset of `subTitle`.
    pub const OFF_SUB_TITLE: u32 = 0x080;
    /// Byte offset of `detail`.
    pub const OFF_DETAIL: u32 = 0x100;
    /// Byte offset of `attribute`.
    pub const OFF_ATTRIBUTE: u32 = 0x500;
    /// Byte offset of `parentalLevel` (alias `reserved2[4]` on FW 3.70+).
    pub const OFF_PARENTAL_LEVEL: u32 = 0x504;
    /// Byte offset of `listParam`.
    pub const OFF_LIST_PARAM: u32 = 0x508;
    /// Total struct size in bytes.
    pub const SIZE: u32 = 0x610;
}

/// `CellSaveDataAutoIndicator` (0x18). Optional progress overlay
/// attached via `statSet->indicator`. Trailing 4 bytes are a
/// reserved `vm::bptr<void>`.
pub mod auto_indicator_layout {
    /// Byte offset of `dispPosition`.
    pub const OFF_DISP_POSITION: u32 = 0x00;
    /// Byte offset of `dispMode`.
    pub const OFF_DISP_MODE: u32 = 0x04;
    /// Byte offset of `dispMsg`.
    pub const OFF_DISP_MSG: u32 = 0x08;
    /// Byte offset of `picBufSize`.
    pub const OFF_PIC_BUF_SIZE: u32 = 0x0C;
    /// Byte offset of `picBuf`.
    pub const OFF_PIC_BUF: u32 = 0x10;
    /// Total struct size in bytes.
    pub const SIZE: u32 = 0x18;
}

/// `CellSaveDataStatGet` (0x6A8). Populated by the system before
/// `funcStat` fires. `fileListNum` is capped by
/// `setBuf->fileListMax`; `sysSizeKB` is fixed at 35 KiB. 64
/// reserved bytes trail `fileList`.
pub mod stat_get_layout {
    /// Byte offset of `hddFreeSizeKB`.
    pub const OFF_HDD_FREE_SIZE_KB: u32 = 0x000;
    /// Byte offset of `isNewData`.
    pub const OFF_IS_NEW_DATA: u32 = 0x004;
    /// Byte offset of embedded `dir` (CellSaveDataDirStat).
    pub const OFF_DIR: u32 = 0x008;
    /// Byte offset of embedded `getParam` (SystemFileParam).
    pub const OFF_GET_PARAM: u32 = 0x040;
    /// Byte offset of `bind`.
    pub const OFF_BIND: u32 = 0x650;
    /// Byte offset of `sizeKB`.
    pub const OFF_SIZE_KB: u32 = 0x654;
    /// Byte offset of `sysSizeKB`.
    pub const OFF_SYS_SIZE_KB: u32 = 0x658;
    /// Byte offset of `fileNum`.
    pub const OFF_FILE_NUM: u32 = 0x65C;
    /// Byte offset of `fileListNum`.
    pub const OFF_FILE_LIST_NUM: u32 = 0x660;
    /// Byte offset of `fileList` (guest pointer to CellSaveDataFileStat array).
    pub const OFF_FILE_LIST: u32 = 0x664;
    /// Total struct size in bytes.
    pub const SIZE: u32 = 0x6A8;
}

/// `CellSaveDataStatSet` (0x0C). `reCreateMode`: 0 keep,
/// 1 keep-no-broken, 2 wipe, 3 wipe-and-reset-owner.
pub mod stat_set_layout {
    /// Byte offset of `setParam` (guest pointer to SystemFileParam).
    pub const OFF_SET_PARAM: u32 = 0x00;
    /// Byte offset of `reCreateMode`.
    pub const OFF_RECREATE_MODE: u32 = 0x04;
    /// Byte offset of `indicator` (guest pointer to AutoIndicator).
    pub const OFF_INDICATOR: u32 = 0x08;
    /// Total struct size in bytes.
    pub const SIZE: u32 = 0x0C;
}

/// `CellSaveDataFileGet` (0x44). 64 reserved bytes trail `excSize`.
pub mod file_get_layout {
    /// Byte offset of `excSize`.
    pub const OFF_EXC_SIZE: u32 = 0x00;
    /// Total struct size in bytes.
    pub const SIZE: u32 = 0x44;
}

/// `CellSaveDataFileSet` (0x30). `secureFileId` is `be_t<u128, 1>`
/// -- align-1 lets it sit immediately after `fileType`.
pub mod file_set_layout {
    /// Byte offset of `fileOperation`.
    pub const OFF_FILE_OPERATION: u32 = 0x00;
    /// Byte offset of `reserved`.
    pub const OFF_RESERVED: u32 = 0x04;
    /// Byte offset of `fileType`.
    pub const OFF_FILE_TYPE: u32 = 0x08;
    /// Byte offset of `secureFileId`.
    pub const OFF_SECURE_FILE_ID: u32 = 0x0C;
    /// Byte offset of `fileName` (guest pointer).
    pub const OFF_FILE_NAME: u32 = 0x1C;
    /// Byte offset of `fileOffset`.
    pub const OFF_FILE_OFFSET: u32 = 0x20;
    /// Byte offset of `fileSize`.
    pub const OFF_FILE_SIZE: u32 = 0x24;
    /// Byte offset of `fileBufSize`.
    pub const OFF_FILE_BUF_SIZE: u32 = 0x28;
    /// Byte offset of `fileBuf` (guest pointer).
    pub const OFF_FILE_BUF: u32 = 0x2C;
    /// Total struct size in bytes.
    pub const SIZE: u32 = 0x30;
}

/// `CellSaveDataSetBuf` (0x28). 24 bytes of reserved padding sit
/// between `fileListMax` and `bufSize`.
pub mod set_buf_layout {
    /// Byte offset of `dirListMax`.
    pub const OFF_DIR_LIST_MAX: u32 = 0x00;
    /// Byte offset of `fileListMax`.
    pub const OFF_FILE_LIST_MAX: u32 = 0x04;
    /// Byte offset of `bufSize`.
    pub const OFF_BUF_SIZE: u32 = 0x20;
    /// Byte offset of `buf` (guest pointer).
    pub const OFF_BUF: u32 = 0x24;
    /// Total struct size in bytes.
    pub const SIZE: u32 = 0x28;
}

const _: () = {
    assert!(error::as_r3(error::NODATA) == 0xFFFF_FFFF_8002_B40B);
    assert!(error::as_r3(error::PARAM) == 0xFFFF_FFFF_8002_B404);
    assert!(error::as_r3(0) == 0);

    assert!(cb_result_layout::OFF_USERDATA + 4 == cb_result_layout::SIZE);
    assert!(dir_stat_layout::OFF_DIR_NAME + size::DIRNAME == dir_stat_layout::SIZE);

    assert!(file_stat_layout::OFF_FILE_TYPE + 8 == file_stat_layout::OFF_SIZE_BYTES);
    assert!(file_stat_layout::OFF_SIZE_BYTES + 8 == file_stat_layout::OFF_ATIME);
    assert!(file_stat_layout::OFF_ATIME + 8 == file_stat_layout::OFF_MTIME);
    assert!(file_stat_layout::OFF_MTIME + 8 == file_stat_layout::OFF_CTIME);
    assert!(file_stat_layout::OFF_CTIME + 8 == file_stat_layout::OFF_FILE_NAME);
    assert!(file_stat_layout::OFF_FILE_NAME + 16 == file_stat_layout::SIZE);

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
    assert!(
        system_file_param_layout::OFF_LIST_PARAM + size::SYSP_LPARAM + 256
            == system_file_param_layout::SIZE
    );

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
    assert!(stat_get_layout::OFF_FILE_LIST + 4 + 64 == stat_get_layout::SIZE);

    assert!(stat_set_layout::OFF_RECREATE_MODE + 4 == stat_set_layout::OFF_INDICATOR);
    assert!(stat_set_layout::OFF_INDICATOR + 4 == stat_set_layout::SIZE);

    assert!(file_get_layout::OFF_EXC_SIZE + 4 + 64 == file_get_layout::SIZE);

    assert!(file_set_layout::OFF_FILE_TYPE + 4 == file_set_layout::OFF_SECURE_FILE_ID);
    assert!(
        file_set_layout::OFF_SECURE_FILE_ID + size::SECUREFILEID == file_set_layout::OFF_FILE_NAME
    );
    assert!(file_set_layout::OFF_FILE_NAME + 4 == file_set_layout::OFF_FILE_OFFSET);
    assert!(file_set_layout::OFF_FILE_OFFSET + 4 == file_set_layout::OFF_FILE_SIZE);
    assert!(file_set_layout::OFF_FILE_SIZE + 4 == file_set_layout::OFF_FILE_BUF_SIZE);
    assert!(file_set_layout::OFF_FILE_BUF_SIZE + 4 == file_set_layout::OFF_FILE_BUF);
    assert!(file_set_layout::OFF_FILE_BUF + 4 == file_set_layout::SIZE);

    assert!(set_buf_layout::OFF_FILE_LIST_MAX + 4 + 24 == set_buf_layout::OFF_BUF_SIZE);
    assert!(set_buf_layout::OFF_BUF_SIZE + 4 == set_buf_layout::OFF_BUF);
    assert!(set_buf_layout::OFF_BUF + 4 == set_buf_layout::SIZE);

    assert!(auto_indicator_layout::OFF_DISP_MODE + 4 == auto_indicator_layout::OFF_DISP_MSG);
    assert!(auto_indicator_layout::OFF_DISP_MSG + 4 == auto_indicator_layout::OFF_PIC_BUF_SIZE);
    assert!(auto_indicator_layout::OFF_PIC_BUF_SIZE + 4 == auto_indicator_layout::OFF_PIC_BUF);
    assert!(auto_indicator_layout::OFF_PIC_BUF + 4 + 4 == auto_indicator_layout::SIZE);
};
