//! `sys_prx` ABI: `CellPrxError` codes, the start/stop option struct
//! layout, and the start-command vocabulary.
//!
//! Symbols and codes mirror RPCS3's `sys_prx.h`. The `CellPrxError`
//! range is disjoint from [`crate::cell_errors`] -- PRX errors are
//! `0x8001_1xxx`, LV2 errnos are `0x8001_0xxx` -- so the two tables
//! never collide.

/// A `CellPrxError` code with its symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrxErrCode {
    /// Numeric code.
    pub code: u32,
    /// Symbol name, e.g. `"CELL_PRX_ERROR_UNKNOWN_MODULE"`.
    pub symbol: &'static str,
}

impl From<PrxErrCode> for u64 {
    fn from(err: PrxErrCode) -> u64 {
        u64::from(err.code)
    }
}

macro_rules! prx_error_table {
    ( $( $name:ident = $code:literal , $desc:literal ; )* ) => {
        $(
            #[doc = $desc]
            pub const $name: PrxErrCode = PrxErrCode {
                code: $code,
                symbol: stringify!($name),
            };
        )*

        /// Every `CellPrxError` entry, ascending by code.
        pub const ENTRIES: &[&PrxErrCode] = &[ $( & $name , )* ];
    };
}

prx_error_table! {
    CELL_PRX_ERROR_ERROR                 = 0x8001_1001, "Unspecified PRX error";
    CELL_PRX_ERROR_ILLEGAL_PERM          = 0x8001_10D1, "No permission for the operation";
    CELL_PRX_ERROR_UNKNOWN_MODULE        = 0x8001_112E, "The specified module id does not exist";
    CELL_PRX_ERROR_ALREADY_STARTED       = 0x8001_1133, "The module is already started";
    CELL_PRX_ERROR_NOT_STARTED           = 0x8001_1134, "The module is not started";
    CELL_PRX_ERROR_ALREADY_STOPPED       = 0x8001_1135, "The module is already stopped";
    CELL_PRX_ERROR_CAN_NOT_STOP          = 0x8001_1136, "The module cannot be stopped";
    CELL_PRX_ERROR_NOT_REMOVABLE         = 0x8001_1138, "The module cannot be unloaded from its current state";
    CELL_PRX_ERROR_LIBRARY_NOT_YET_LINKED = 0x8001_113A, "The library is not yet linked";
    CELL_PRX_ERROR_LIBRARY_FOUND         = 0x8001_113B, "The library is already registered";
    CELL_PRX_ERROR_LIBRARY_NOTFOUND      = 0x8001_113C, "The library is not registered";
    CELL_PRX_ERROR_ILLEGAL_LIBRARY       = 0x8001_113D, "The library structure is invalid";
    CELL_PRX_ERROR_LIBRARY_INUSE         = 0x8001_113E, "The library is still in use";
    CELL_PRX_ERROR_ALREADY_STOPPING      = 0x8001_113F, "The module is already stopping";
    CELL_PRX_ERROR_UNSUPPORTED_PRX_TYPE  = 0x8001_1148, "Unsupported PRX type";
    CELL_PRX_ERROR_INVAL                 = 0x8001_1324, "Invalid argument";
    CELL_PRX_ERROR_ILLEGAL_PROCESS       = 0x8001_1801, "The process is not permitted the operation";
    CELL_PRX_ERROR_NO_LIBLV2             = 0x8001_1881, "liblv2 is absent";
    CELL_PRX_ERROR_UNSUPPORTED_ELF_TYPE  = 0x8001_1901, "Unsupported ELF type";
    CELL_PRX_ERROR_UNSUPPORTED_ELF_CLASS = 0x8001_1902, "Unsupported ELF class";
    CELL_PRX_ERROR_UNDEFINED_SYMBOL      = 0x8001_1904, "An imported symbol is undefined";
    CELL_PRX_ERROR_UNSUPPORTED_RELOCATION_TYPE = 0x8001_1905, "Unsupported relocation type";
    CELL_PRX_ERROR_ELF_IS_REGISTERED     = 0x8001_1910, "The ELF is already registered";
    CELL_PRX_ERROR_NO_EXIT_ENTRY         = 0x8001_1911, "The module has no exit entry";
}

/// `sys_prx_start_stop_module_option_t` field offsets.
///
/// Every field is 8 bytes wide and big-endian. `size` being a `u64`
/// is the trap: reading only its low 4 bytes at offset 0 yields the
/// HIGH half, which is zero for every realistic size.
pub mod start_stop_option {
    /// `be_t<u64> size` -- total struct size in bytes.
    pub const SIZE_OFFSET: u32 = 0x00;
    /// `be_t<u64> cmd` -- phase selector; only the low nibble is read.
    pub const CMD_OFFSET: u32 = 0x08;
    /// `be_t<u64> entry` -- OUT: the entry the caller should invoke.
    pub const ENTRY_OFFSET: u32 = 0x10;
    /// `be_t<u64> res` -- IN on cmd 2: what the entry returned.
    pub const RES_OFFSET: u32 = 0x18;
    /// `be_t<u64> entry2` -- OUT, present only when `size != 0x20`.
    pub const ENTRY2_OFFSET: u32 = 0x20;

    /// Smallest legal struct: through `res`, no `entry2`.
    pub const MIN_SIZE: u64 = 0x20;

    /// Sentinel written to `entry` when there is nothing to call.
    pub const NO_ENTRY: u64 = u64::MAX;
}

/// Low nibble of `sys_prx_start_stop_module_option_t::cmd`.
pub mod start_cmd {
    /// Phase 1: hand the caller the entry to invoke.
    pub const GET_ENTRY: u64 = 1;
    /// Phase 2: report what the entry returned via `res`.
    pub const REPORT_RESULT: u64 = 2;
    /// Only the low nibble selects the phase.
    pub const MASK: u64 = 0xF;
}

/// `res` value meaning the started module stays resident.
pub const SYS_PRX_RESIDENT: u64 = 0;
