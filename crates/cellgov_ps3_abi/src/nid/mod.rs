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
