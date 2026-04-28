//! cellVideoOut PS3 ABI: display-state and resolution enums + error codes.
//!
//! Mirrors the layout of RPCS3's
//! `rpcs3/Emu/Cell/Modules/cellVideoOut.h`. Behaviour (handlers,
//! dispatch) lives in `cellgov_core::hle::cellSysutil` (the NIDs are
//! exported by the cellSysutil module per the PS3 NID database, even
//! though the data definitions sit in the cellVideoOut header).

/// `videoOut` index (`CELL_VIDEO_OUT_PRIMARY` / `_SECONDARY`, u32).
pub mod display {
    /// Primary display port.
    pub const PRIMARY: u32 = 0;
    /// Secondary display port (CellGov reports zero devices on this).
    pub const SECONDARY: u32 = 1;
}

/// `CellVideoOutDisplayMode::state` (u8).
pub mod output_state {
    /// Display attached and reporting a valid mode.
    pub const ENABLED: u8 = 0;
}

/// `CellVideoOutColorSpace` (u8).
pub mod color_space {
    /// RGB color space (the only one CellGov reports).
    pub const RGB: u8 = 0x01;
}

/// `CellVideoOutResolutionId` as the u32 caller hands to
/// `cellVideoOutGetResolution`.
pub mod resolution_id {
    /// 1920x1080 (1080p).
    pub const ID_1080: u32 = 1;
    /// 1280x720 (720p).
    pub const ID_720: u32 = 2;
    /// 720x480 (480p / NTSC).
    pub const ID_480: u32 = 4;
    /// 720x576 (576p / PAL).
    pub const ID_576: u32 = 5;
    /// 1600x1080.
    pub const ID_1600X1080: u32 = 6;
    /// 1440x1080.
    pub const ID_1440X1080: u32 = 7;
    /// 1280x1080.
    pub const ID_1280X1080: u32 = 8;
    /// 960x1080.
    pub const ID_960X1080: u32 = 10;
}

/// `CellVideoOutDisplayMode::resolutionId` (u8 form of the same enum).
pub mod display_mode_resolution {
    /// 1280x720 (720p).
    pub const ID_720: u8 = 2;
}

/// `CellVideoOutDisplayMode::scanMode` (u8).
pub mod scan_mode {
    /// Progressive scan.
    pub const PROGRESSIVE: u8 = 1;
}

/// `CellVideoOutDisplayMode::conversion` (u8).
pub mod display_conversion {
    /// No conversion applied.
    pub const NONE: u8 = 0;
}

/// `CellVideoOutDisplayMode::aspect` (u8).
pub mod aspect {
    /// 16:9 widescreen.
    pub const WIDE_16_9: u8 = 2;
}

/// `CellVideoOutDisplayMode::refreshRates` bitfield (u16).
pub mod refresh_rate {
    /// 59.94 Hz (NTSC-derived).
    pub const HZ_59_94: u16 = 0x0001;
}

/// `CELL_VIDEO_OUT_ERROR_*` band (`0x8002_b2xx`).
pub mod error {
    /// Invalid argument (null guest out-pointer, etc.).
    pub const ILLEGAL_PARAMETER: u32 = 0x8002_b222;
    /// `deviceIndex` out of range for the chosen `videoOut`.
    pub const DEVICE_NOT_FOUND: u32 = 0x8002_b224;
    /// `videoOut` index out of range.
    pub const UNSUPPORTED_VIDEO_OUT: u32 = 0x8002_b225;
}
