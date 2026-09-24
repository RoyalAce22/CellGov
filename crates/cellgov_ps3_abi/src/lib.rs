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
        clippy::disallowed_macros,
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

pub(crate) use nid::nid_module;
