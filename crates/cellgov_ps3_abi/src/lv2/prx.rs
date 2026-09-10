//! `sys_prx` ABI: `CellPrxError` codes, the start/stop option struct
//! layout, and the start-command vocabulary. The `CellPrxError` range
//! is disjoint from [`crate::lv2::errno`] -- PRX errors are
//! `0x8001_1xxx`, LV2 errnos are `0x8001_0xxx` -- so the two tables
//! never collide.
//!
//! Only the codes CellGov answers with appear here. Installed firmware
//! materialises four of the eight in its own code or data; the other
//! four carry the symbol-to-value pairing alone, with no firmware
//! witness.
//!
//! liblv2.sprx issues sc 481 and 482 and builds the option struct on
//! its own stack, so the offsets, the declared size and the command
//! values below are read off that caller.

/// The module id does not name a loaded module.
///
/// libfiber.sprx and libsre.sprx build this constant with `lis` / `ori`
/// before returning it.
pub const CELL_PRX_ERROR_UNKNOWN_MODULE: u32 = 0x8001_112E;
/// The module is not started.
pub const CELL_PRX_ERROR_NOT_STARTED: u32 = 0x8001_1134;
/// The module cannot be unloaded from its current state.
pub const CELL_PRX_ERROR_NOT_REMOVABLE: u32 = 0x8001_1138;
/// The module is already stopping.
pub const CELL_PRX_ERROR_ALREADY_STOPPING: u32 = 0x8001_113F;
/// Unspecified PRX error; the start/stop handlers answer it for a
/// command word they do not recognise.
pub const CELL_PRX_ERROR_ERROR: u32 = 0x8001_1001;
/// The module is already stopped.
pub const CELL_PRX_ERROR_ALREADY_STOPPED: u32 = 0x8001_1135;
/// The module refused to stop.
pub const CELL_PRX_ERROR_CAN_NOT_STOP: u32 = 0x8001_1136;
/// The ELF is already registered.
pub const CELL_PRX_ERROR_ELF_IS_REGISTERED: u32 = 0x8001_1910;

/// `sys_prx_start_stop_module_option_t` field offsets.
///
/// Five fields, each 8 bytes wide and big-endian. `size` is a `u64`:
/// a reader that takes only its low 4 bytes at offset 0 gets the HIGH
/// half, which is zero for every realistic size.
///
/// liblv2.sprx declares `size = 0x28` -- all five fields -- on every
/// sc 481 / 482 it issues, so `MIN_SIZE` is a floor no firmware caller
/// presents.
pub mod start_stop_option {
    /// `size` -- total struct size in bytes.
    pub const SIZE_OFFSET: u32 = 0x00;
    /// `cmd` -- phase selector.
    pub const CMD_OFFSET: u32 = 0x08;
    /// `entry` -- OUT: an entry the caller should invoke.
    pub const ENTRY_OFFSET: u32 = 0x10;
    /// `res` -- IN on the report phase: the `s32` the entry returned,
    /// sign-extended to 64 bits by the caller.
    pub const RES_OFFSET: u32 = 0x18;
    /// `entry2` -- OUT, present only when `size != 0x20`. liblv2.sprx
    /// invokes this one first and falls back to `entry`.
    pub const ENTRY2_OFFSET: u32 = 0x20;

    /// Smallest legal struct: through `res`, no `entry2`.
    pub const MIN_SIZE: u64 = 0x20;

    /// Sentinel meaning "nothing to invoke".
    ///
    /// liblv2.sprx seeds `entry2` with it and skips its indirect call
    /// while both slots still hold it, so a slot the kernel leaves
    /// untouched reads as this value. With neither slot callable, the
    /// start path reports `res = 0` and the teardown path gives up
    /// with `0x8001_1911` (`CELL_PRX_ERROR_NO_EXIT_ENTRY`) and never
    /// reaches the report phase.
    pub const NO_ENTRY: u64 = u64::MAX;
}

/// `sys_prx_get_module_list_option_t`.
pub mod get_module_list_option {
    /// The layout liblv2.sprx declares on every call it issues.
    pub const SIZE: u64 = 0x20;
}

/// Low nibble of `sys_prx_start_stop_module_option_t::cmd`.
///
/// liblv2.sprx's `sys_prx_start_module` writes 1, invokes whatever
/// came back, then writes 2 with the result -- two syscalls behind one
/// guest-visible call. Its `sys_prx_stop_module` runs the same pair
/// over sc 482.
pub mod start_cmd {
    /// Phase 1: hand the caller the entry to invoke.
    pub const GET_ENTRY: u64 = 1;
    /// Phase 2: report what the entry returned via `res`.
    pub const REPORT_RESULT: u64 = 2;
    /// Phase-selector mask. Every `cmd` liblv2.sprx writes fits the
    /// nibble, so no firmware caller distinguishes masking from an
    /// exact-value match.
    pub const MASK: u64 = 0xF;
}

/// `cmd` values specific to `_sys_prx_stop_module`; phases 1 / 2 are
/// shared with [`start_cmd`].
///
/// 4 and 8 are the second matched pair: liblv2.sprx's teardown helper,
/// reached from `sys_prx_exitspawn_with_level`, asks with 4 and reports
/// with 8 instead of 1 and 2.
pub mod stop_cmd {
    /// Teardown phase 1: hand back the stop entries.
    pub const GET_ENTRIES: u64 = 4;
    /// Teardown phase 2: report what the stop entry returned.
    pub const REPORT_ENTRIES_RESULT: u64 = 8;
}

/// `res` value meaning the started module stays resident.
pub const SYS_PRX_RESIDENT: u64 = 0;
