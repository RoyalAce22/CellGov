//! `sys_prx` ABI: `CellPrxError` codes, the start/stop option struct
//! layout, the start-command vocabulary, and the module-name size. The
//! `CellPrxError` range is disjoint from [`crate::lv2::errno`] -- PRX
//! errors are `0x8001_1xxx`, LV2 errnos are `0x8001_0xxx` -- so the two
//! tables never collide.
//!
//! Only the codes CellGov answers with appear here. Each one names
//! what shows its value: a capture from the reference console, or the
//! firmware code that builds or compares it.
//!
//! liblv2.sprx issues sc 481 and 482 and builds the option struct on
//! its own stack, so the offsets, the declared size and the command
//! values below are read off that caller.

/// The module id does not name a loaded module.
///
/// libfiber.sprx and libsre.sprx build this constant with `lis` / `ori`
/// before returning it, and the `lv2_prx_module_id_by_name` capture
/// answers it for a name no module carries.
pub const CELL_PRX_ERROR_UNKNOWN_MODULE: u32 = 0x8001_112E;

/// The most bytes of a module name `_sys_prx_get_module_id_by_name`
/// reads and compares: the size of the module-info name field.
// The oracle's prx module reads and compares this many.
pub const MODULE_NAME_MAX: usize = 28;
/// The module is not started.
///
/// The `lv2_prx_states` capture answers it to a stop of a module that
/// never started.
pub const CELL_PRX_ERROR_NOT_STARTED: u32 = 0x8001_1134;
/// The module cannot be unloaded from its current state.
///
/// The `lv2_prx_states` capture answers it to an unload between the
/// two start phases, and `lv2_prx_states_last` to an unload of a
/// started module.
pub const CELL_PRX_ERROR_NOT_REMOVABLE: u32 = 0x8001_1138;
/// The module is already stopping.
///
/// The `lv2_prx_states_more` capture answers it to a second stop of a
/// stopping module.
pub const CELL_PRX_ERROR_ALREADY_STOPPING: u32 = 0x8001_113F;
/// Unspecified PRX error; the start/stop handlers answer it for a
/// command word they do not recognise.
///
/// The `lv2_prx_states` capture answers it to start commands 3, 0x10
/// and 0.
pub const CELL_PRX_ERROR_ERROR: u32 = 0x8001_1001;
/// The module is already stopped.
///
/// The `lv2_prx_states_more` capture answers it to a stop of a stopped
/// module.
pub const CELL_PRX_ERROR_ALREADY_STOPPED: u32 = 0x8001_1135;
/// The module refused to stop.
///
/// The `lv2_prx_states_more` capture answers it to a stop report of 1.
pub const CELL_PRX_ERROR_CAN_NOT_STOP: u32 = 0x8001_1136;
/// The ELF is already registered.
///
/// liblv2.sprx's own import-bind call compares its answer against this
/// value and takes it as success.
pub const CELL_PRX_ERROR_ELF_IS_REGISTERED: u32 = 0x8001_1910;

/// A `CellPrxError` code as a syscall returns it in r3: sign-extended
/// to 64 bits, as every error code is. The `lv2_prx_states` capture
/// reads `0xFFFF_FFFF_8001_1134` for `CELL_PRX_ERROR_NOT_STARTED`.
pub const fn return_code(code: u32) -> u64 {
    code as i32 as i64 as u64
}

/// The bit of a module id `_sys_prx_start_module` refuses with
/// `CELL_EINVAL` before the lookup: the id read as a signed 32-bit value
/// is negative. `_sys_prx_stop_module` looks such an id up instead (the
/// `lv2_prx_start_gate` capture).
pub const PRX_ID_SIGN_BIT: u32 = 0x8000_0000;

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

    // The sc 481 / 482 arms gate `pOpt + MIN_SIZE` against a 32-bit
    // wrap. Once `size` names the extended form, they gate
    // `pOpt + ENTRY2_OFFSET + 8` as well. They form every field
    // address without a further check, so each gate has to reach the
    // end of the last field it covers.
    const _: () = assert!(RES_OFFSET as u64 + 8 == MIN_SIZE);
    const _: () = assert!(CMD_OFFSET < RES_OFFSET && ENTRY_OFFSET < RES_OFFSET);
    const _: () = assert!(SIZE_OFFSET == 0);
    const _: () = assert!(ENTRY2_OFFSET as u64 == MIN_SIZE);
}

/// `sys_prx_get_module_list_option_t`.
///
/// liblv2.sprx's `sys_prx_get_module_list` builds the struct on its own
/// stack, so the offsets and the declared size are read off that
/// caller:
///
/// - `size@0`, a `u64`
/// - `pad@8`, which no caller writes
/// - `max@0xC`
/// - `count@0x10`
/// - `idlist@0x14`
/// - `unk@0x18`
/// - tail padding to [`get_module_list_option::SIZE`]
///
/// The `pad` and `unk` words carry no name in that caller. With both
/// zero the reference console fills the list; with `0xAA` in every byte
/// of them and of `count`, it stopped answering
/// (`tests/micro/console_crashes.md`), so the kernel reads at least one
/// of them. What they hold is unestablished.
///
/// The `lv2_prx_module_list_sizes` capture accepts a declared size of
/// 0x18 as well as liblv2.sprx's 0x20 and fills the same offsets: 0x18
/// ends at the `idlist` word. It refuses 0, 0x1C, 0x24, 0x28, 0x30 and
/// 0x40 with `CELL_EINVAL`.
pub mod get_module_list_option {
    /// The layout liblv2.sprx declares on every call it issues.
    pub const SIZE: u64 = 0x20;

    /// Every declared size the kernel accepts: [`SIZE`] and the form
    /// that stops at the end of `idlist`.
    pub const ACCEPTED_SIZES: [u64; 2] = [TOUCHED_LEN as u64, SIZE];

    /// `max` -- IN: slots the caller's `idlist` array holds.
    pub const MAX_OFFSET: u32 = 0x0C;
    /// `count` -- OUT: modules the kernel filled in.
    pub const COUNT_OFFSET: u32 = 0x10;
    /// `idlist` -- IN: guest pointer to the `u32` id array.
    pub const IDLIST_OFFSET: u32 = 0x14;

    /// Bytes of the struct the kernel reads or writes. The kernel
    /// never touches the tail from here to [`SIZE`], so a caller that
    /// maps only this much still gets a complete call.
    pub const TOUCHED_LEN: u32 = 0x18;

    /// Width of one `idlist` entry: a big-endian module id.
    pub const ID_SIZE: u32 = 4;
    /// Width of the `count` field.
    pub const COUNT_SIZE: u32 = 4;

    /// `flags` bit that asks for the module list.
    ///
    /// liblv2.sprx passes `flags = 2` on every call. The
    /// `lv2_prx_module_list` capture fills the list for flags 2 and 3
    /// and answers CELL_OK without a fill for 0, 1 and 4.
    pub const FLAG_FILL_LIST: u64 = 0x2;

    // The sc 494 arm gates `pInfo + TOUCHED_LEN` against a 32-bit
    // wrap once, then forms every field address without a further
    // check. The gate has to reach the end of the last field it reads.
    const _: () = assert!(MAX_OFFSET + COUNT_SIZE <= TOUCHED_LEN);
    const _: () = assert!(COUNT_OFFSET + COUNT_SIZE <= TOUCHED_LEN);
    const _: () = assert!(IDLIST_OFFSET + ID_SIZE <= TOUCHED_LEN);
    const _: () = assert!(TOUCHED_LEN as u64 <= SIZE);
}

/// `sys_prx_register_module_option_t`.
///
/// `size@0` selects the form. [`register_module_option::SIZE`] is the
/// only form that carries `type@8`, `stub_ea@0x20` and
/// `stub_size@0x24`. The two forms in
/// [`register_module_option::LEGACY_SIZES`] stop short of all three.
///
/// The layout is unestablished: nothing in the captures or the public
/// documents fixes the offsets or the three accepted sizes. Once a
/// game process is running, the `lv2_prx_register` capture answers it
/// `CELL_EAGAIN` for every declared size before a field is read; what
/// liblv2.sprx's own calls during process start get is unprobed. Only
/// a CoreOS caller's own code, which shows the fields it writes before
/// the call, can fix the layout.
pub mod register_module_option {
    /// The form that carries the type word and the stub-table pair.
    pub const SIZE: u64 = 0x30;

    /// Forms that stop short of `type@8`.
    ///
    /// A caller that presents one asks for no import binding, so the
    /// kernel reads none of the fields past `size`.
    pub const LEGACY_SIZES: [u64; 2] = [0x1C, 0x20];

    /// `type` -- IN: bit 0 asks the kernel to bind the caller's own
    /// import tables. The remaining bits have no witness.
    pub const TYPE_OFFSET: u32 = 0x08;
    /// `stub_ea` -- IN: guest address of the import table.
    pub const STUB_EA_OFFSET: u32 = 0x20;
    /// `stub_size` -- IN: bytes of that table.
    pub const STUB_SIZE_OFFSET: u32 = 0x24;

    /// Bit of `type` that asks for the import binding.
    pub const TYPE_MANUAL_IMPORTS: u64 = 0x1;

    /// Bytes through the end of `stub_size`, the last field any form
    /// reads. The tail from here to [`SIZE`] stays untouched.
    pub const TOUCHED_LEN: u32 = 0x28;

    // The sc 484 arm gates `pOpt + TOUCHED_LEN` against a 32-bit wrap
    // once, then forms every field address without a further check.
    // The gate has to reach the end of the last field it reads.
    const _: () = assert!(TYPE_OFFSET + core::mem::size_of::<u64>() as u32 <= TOUCHED_LEN);
    const _: () = assert!(STUB_EA_OFFSET + core::mem::size_of::<u32>() as u32 <= TOUCHED_LEN);
    const _: () = assert!(STUB_SIZE_OFFSET + core::mem::size_of::<u32>() as u32 == TOUCHED_LEN);
    const _: () = assert!(TOUCHED_LEN as u64 <= SIZE);
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
    /// Phase-selector mask. The `lv2_prx_load_cycle` capture sends
    /// `cmd = 0x11` to a started module's stop and gets phase 1's
    /// answer. The `lv2_prx_states` capture answers start commands
    /// 0x11 and 0x21 as 1, start commands 3, 0x10 and 0 as unknown,
    /// and stop command 0x14 as 4, so the kernel masks rather than
    /// matching the value.
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
///
/// The `lv2_prx_states_more` and `lv2_prx_states_last` captures keep a
/// module whose start report is 0, and withdraw one whose report is 1
/// or 5, answering the report itself.
pub const SYS_PRX_RESIDENT: u64 = 0;
