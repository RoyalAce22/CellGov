//! PS3 / Cell / PowerPC external-ABI facts and the pure functions over
//! them. It holds no guest state, does no I/O, and depends on no
//! workspace crate, so any workspace crate may depend on it.
//!
//! - NIDs, each checked against its SHA-1 at compile time
//! - error codes
//! - syscall numbers and the namespace layout over them
//! - struct offsets and flag bits
//! - binary-format and hardware constants
//! - the PPC64 encoders for CellGov's own stubs

#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(
    not(test),
    forbid(
        clippy::disallowed_methods,
        clippy::print_stdout,
        clippy::print_stderr,
        clippy::dbg_macro
    )
)]

pub mod codegen;
pub mod format;
pub mod hw;
pub mod lv2;
pub mod nid;
pub mod sha1;

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

/// Declares one PS3 library's NIDs as SHA-1-verified `pub const`s.
///
/// The macro also emits the module's `DECLARED_NIDS` slice, one row
/// per NID. `nid::CURATED` lists that slice, and the `nid` table tests
/// reconcile it against `NID_TABLE`.
macro_rules! nid_module {
    ( $( $name:ident = $value:expr, $fn:literal; )* ) => {
        $(
            $crate::nid_const!($name = $value, $fn);
        )*

        /// Every NID this module declares, paired with the guest
        /// function name behind its literal.
        pub const DECLARED_NIDS: &[(u32, &str)] = &[
            $( ($name, $fn), )*
        ];
    };
}

pub(crate) use nid_module;
