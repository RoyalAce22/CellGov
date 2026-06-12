//! PS3 ABI constants: NIDs, error codes, struct offsets, and flag bits
//! shared across workspace crates without inducing backward DAG edges.

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod cell_errors;
pub mod elf;
pub mod hardware;
pub mod nid;
pub mod ppc_isa;
pub mod process_address_space;
pub mod sce;
pub mod sha1;
pub mod spu_channels;
pub mod sys_fs;
pub mod sys_memory;
pub mod sys_process;
pub mod sys_rsx;
pub mod sys_spu;
pub mod sys_sync;
pub mod syscall;
pub mod syscall_namespace;
pub mod system_ipc;
pub mod trampoline_codegen;

pub mod rsx_nv_hardware;

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

/// Declare a PS3 PRX library NID module from a single declarative
/// source. Each named NID becomes a SHA-1-verified `pub const` (via
/// [`nid_const!`]); the `classified { ... }` block additionally
/// contributes to a per-module `CLASSIFIED_NIDS: &[u32]` slice that
/// `nid::tests::every_classified_nid_has_explicit_arm` walks to
/// enforce the "every classified NID has an explicit arm in
/// `stub_classification_explicit`" contract.
///
/// The optional `unclassified { ... }` block emits the same per-NID
/// `pub const` declarations but does NOT include them in
/// `CLASSIFIED_NIDS`; use it for NIDs defined at a typed callsite
/// that have not yet been reviewed for a stub-class verdict. Such
/// NIDs keep surfacing through the unclaimed-NID log path until a
/// per-NID review moves them into `classified`.
macro_rules! nid_module {
    (
        classified {
            $( $cname:ident = $cvalue:expr, $cfn:literal; )*
        }
        $(
            unclassified {
                $( $uname:ident = $uvalue:expr, $ufn:literal; )*
            }
        )?
    ) => {
        $(
            $crate::nid_const!($cname = $cvalue, $cfn);
        )*
        $(
            $(
                $crate::nid_const!($uname = $uvalue, $ufn);
            )*
        )?

        /// NIDs grouped under this module that must classify
        /// explicitly in `cellgov_ps3_abi::nid::stub_classification_explicit`
        /// (i.e. NOT fall to the default `NoopSafe` catch-all). The
        /// test `nid::tests::every_classified_nid_has_explicit_arm`
        /// walks every module's slice to enforce the contract.
        ///
        /// Consulted by `cellgov_cli dump-prx-imports` when
        /// classifying unresolved-or-zero-bound PRX imports.
        pub const CLASSIFIED_NIDS: &[u32] = &[ $( $cname ),* ];
    };
}

pub(crate) use nid_module;
