//! PS3 ABI constants: NIDs, error codes, struct offsets, and flag bits
//! shared across workspace crates without inducing backward DAG edges.

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod callback_dispatch;
pub mod cell_errors;
pub mod elf;
pub mod hardware;
pub mod nid;
pub mod process_address_space;
pub mod sha1;
pub mod spu_channels;
pub mod sys_fs;
pub mod sys_memory;
pub mod sys_process;
pub mod sys_rsx;
pub mod sys_spu;
pub mod syscall;
pub mod syscall_namespace;
pub mod trampoline_codegen;

#[path = "sprx_modules/cellGcm.rs"]
pub mod cell_gcm;
#[path = "sprx_modules/cellGcmSys.rs"]
pub mod cell_gcm_sys;
#[path = "sprx_modules/cellSaveData.rs"]
pub mod cell_save_data;
#[path = "sprx_modules/cellSpurs.rs"]
pub mod cell_spurs;
#[path = "sprx_modules/cellVideoOut.rs"]
pub mod cell_video_out;

/// Declares a NID constant whose hex literal is verified against
/// `SHA-1(name || salt)` at compile time.
///
/// ```ignore
/// nid_const!(INITIALIZE = 0xacfc_8dbc, "cellSpursInitialize");
/// ```
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
