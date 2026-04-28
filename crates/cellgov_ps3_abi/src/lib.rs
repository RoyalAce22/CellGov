//! PS3 ABI source-of-truth constants.
//!
//! Holds the NID values, error codes, struct field offsets, and flag
//! bits that are defined by the PS3 system libraries and consumed by
//! multiple workspace crates. The crate is data only and depends on
//! nothing in the workspace, so any crate that needs a PS3 ABI value
//! can import it without inducing a backward DAG edge.
//!
//! Per-PS3-PRX-module ABI data lives under `sprx_modules/` with files
//! named after the original Sony library (camelCase: `cellSpurs.rs`,
//! `cellGcmSys.rs`, etc.) so a `git grep` against this crate lines up
//! one-to-one with `cellgov_core/src/hle/`. Cross-cutting data
//! (errnos, ELF/PRX layout, hardware constants, syscall numbers, NID
//! lookup) lives in flat snake_case files at the top level.
//!
//! No syscall handlers, no effect plumbing, no formatting helpers. See
//! `docs/dev/optimizations/centralized_ps3_abi_crate.md` for the full
//! scope and migration plan.

pub mod cell_errors;
pub mod elf;
pub mod hardware;
pub mod nid;
pub mod sha1;
pub mod sys_memory;
pub mod sys_rsx;
pub mod sys_spu;
pub mod syscall;

// Per-PS3-PRX-module ABI data. Filenames mirror the Sony library
// names; module identifiers stay snake_case to match the rest of the
// crate's path conventions.
#[path = "sprx_modules/cellGcm.rs"]
pub mod cell_gcm;
#[path = "sprx_modules/cellGcmSys.rs"]
pub mod cell_gcm_sys;
#[path = "sprx_modules/cellSpurs.rs"]
pub mod cell_spurs;
#[path = "sprx_modules/cellVideoOut.rs"]
pub mod cell_video_out;

/// Declare a NID constant whose hex literal is verified against
/// `SHA-1(name || salt)` at compile time.
///
/// ```ignore
/// nid_const!(INITIALIZE = 0xacfc_8dbc, "cellSpursInitialize");
/// ```
///
/// expands to a `pub const INITIALIZE: u32 = 0xacfc_8dbc;` and a
/// `const _: () = assert!(nid_sha1("cellSpursInitialize") == 0xacfc_8dbc)`.
/// A wrong literal, a typo'd name, or a salt-derivation drift trips
/// the const-assert at compile time and the offending registration is
/// named in the diagnostic.
#[macro_export]
macro_rules! nid_const {
    ($name:ident = $literal:expr, $fn_name:literal) => {
        #[doc = concat!("NID for guest function `", $fn_name, "`.")]
        pub const $name: u32 = $literal;
        const _: () = assert!(
            $crate::sha1::nid_sha1($fn_name) == $literal,
            concat!(
                "nid_const!: literal does not match SHA-1(\"",
                $fn_name,
                "\" || salt)",
            ),
        );
    };
}
