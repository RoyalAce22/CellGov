//! NID -> `(module, function)` lookup for PS3 system libraries.
//!
//! Per the PS3 NID algorithm, a NID is the first 4 bytes (little-endian
//! u32) of SHA-1(name || suffix) with the named-export suffix
//! `0x6759659904250490566427499489741A`; [`crate::sha1::nid_sha1`] is
//! that derivation, and `table_tests` recomputes every `NID_TABLE` row
//! through it. Two kinds of row do not recompute: a row whose real
//! name is unknown carries the placeholder `<module>_<NID:08X>`, and
//! the module entry points (`module_start`, `module_stop`,
//! `module_exit`, `module_info`, `module_prologue`, `module_epilogue`)
//! carry fixed NIDs the derivation does not produce. NIDs are
//! game-independent.
//!
//! The export tables of the 4.93 firmware modules hold the derivation.
//! They were read from the 142 decrypted modules of
//! `dev_flash/sys/external`. The reference console's `liblv2`, `libfs`,
//! `libsysutil`, `libgcm_sys` and `libsre` are byte-identical to those
//! files. Ten names derived through the salt give the exact NID their
//! library exports: `cellFsOpen`, `cellFsRead`, `sys_lwmutex_create`,
//! `cellGcmGetConfiguration`, `cellSpursInitialize`,
//! `cellVideoOutGetState`, `cellSaveDataAutoLoad`,
//! `sys_ppu_thread_create`, `_sys_malloc` and
//! `cellSysutilCheckCallback`.
//!
//! The entry-point NIDs appear only in a module's nameless export
//! library, whose attribute is 0x8000. None appears in a named library.
//! In those 142 modules:
//!
//! - `module_info` (0xD7F43016) is a variable in every module. It
//!   points at the module's library-information record, the same
//!   record segment 0's `p_paddr` names.
//! - `module_start` (0xBC9A0086, 105 modules), `module_stop`
//!   (0xAB779874, 106) and `module_exit` (0x3AB9A95E, 41) are
//!   functions. Each points at an 8-byte (code, TOC) descriptor; in
//!   `libfs`, `module_start` names code 0x0 and `module_stop` code 0x50.
//! - `module_prologue` (0x0D10FD3F) and `module_epilogue` (0x330F7005)
//!   are functions exported by `librudp` alone.
//! - A seventh nameless-library NID, 0x6D0F6CAE, is a variable in 131
//!   modules; its name is not known.
//!
//! The curated `nid_module!` blocks live in `modules`, listed by name
//! in [`CURATED`]. The full table is `table.rs`; `table_gen_tests`
//! renders it from the `table.tsv` data file beside it and fails on
//! drift.

mod macros;
mod modules;
mod table;

pub(crate) use macros::nid_module;
pub use modules::{
    cell_gcm_sys, cell_save_data, cell_spurs, cell_sysutil, sys_fs, sys_prx_for_user, CURATED,
};
use table::NID_TABLE;

/// Returns `Some((module, function))` if the NID is known. `module` may
/// be the empty string for symbols that ship outside any named PS3
/// library (libstdc++ mangled names, libm helpers, etc.); the caller
/// can treat `("", _)` as "name resolved, module unknown".
pub fn lookup(nid: u32) -> Option<(&'static str, &'static str)> {
    NID_TABLE
        .binary_search_by_key(&nid, |entry| entry.0)
        .ok()
        .map(|i| (NID_TABLE[i].1, NID_TABLE[i].2))
}

#[cfg(test)]
#[path = "tests/nid_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/table_tests.rs"]
mod table_tests;

#[cfg(test)]
#[path = "tests/table_gen_tests.rs"]
mod table_gen_tests;

#[cfg(test)]
#[path = "tests/curated_tests.rs"]
mod curated_tests;
