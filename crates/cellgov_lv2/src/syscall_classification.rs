//! LEV-aware dispatch-hint classifier for `sc` yields.
//!
//! Composes `cellgov_ps3_abi::syscall_namespace::SyscallNamespace::of`
//! (a pure ABI fact: which namespace bucket does an r11 value land
//! in) with the hypercall guard (a routing decision: any non-zero
//! LEV is rejected before LV2 dispatch). The classifier produces
//! [`SyscallClassification`] which the runtime consumes to drive
//! [`crate::Lv2Request`] construction.
//!
//! Lives in `cellgov_lv2` rather than `cellgov_ps3_abi` because
//! the output type is a routing hint, not an ABI fact. The pure
//! namespace functions stay in the ABI crate.

use cellgov_ps3_abi::syscall_namespace::{CellGovPrivateSyscall, SyscallNamespace};

/// Typed classification of an `sc` yield.
///
/// Each variant carries the per-namespace decoded form so the
/// caller does not re-walk the range tables. `Hypercall` and
/// `Unknown` are the two fault paths; the runtime routes them away
/// from LV2 dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SyscallClassification {
    /// LEV=0, r11 in `Lv2` range. The runtime's LV2 dispatcher
    /// receives the raw `number`.
    Lv2 {
        /// r11 verbatim.
        number: u64,
    },
    /// LEV=0, r11 in `HleImport` range. `index` is the per-namespace
    /// 0-based index (the original `hle_index` the binder passed to
    /// `SyscallNamespace::HleImport.encode`).
    HleImport {
        /// 0-based index inside the `HleImport` namespace.
        index: u32,
    },
    /// LEV=0, r11 in `CellGovPrivate` range, recognized variant.
    CellGovPrivate(CellGovPrivateSyscall),
    /// LEV >= 1. PS3 usermode should never issue this; programming
    /// error per Book I §2.4.2. The runtime rejects rather than
    /// silently treating it as LV2.
    Hypercall {
        /// LEV value as decoded from the `sc` instruction.
        lev: u8,
        /// r11 verbatim, preserved for diagnostics.
        r11: u64,
    },
    /// LEV=0 but r11 is above every declared namespace OR inside
    /// `CellGovPrivate` at an index without a registered variant.
    /// Falls through to [`crate::Lv2Request::Unsupported`].
    Unknown {
        /// r11 verbatim.
        r11: u64,
    },
}

/// LEV-aware classification of an `sc` yield.
///
/// Combines the LEV field of the `sc` instruction (Book III §2.3.1)
/// with the r11 syscall number to produce a typed dispatch hint.
/// Non-zero LEV is routed to [`SyscallClassification::Hypercall`]
/// regardless of r11; the runtime rejects these before LV2 dispatch
/// (PS3 usermode programming error per Book I §2.4.2).
#[inline]
pub const fn classify(lev: u8, r11: u64) -> SyscallClassification {
    if lev != 0 {
        return SyscallClassification::Hypercall { lev, r11 };
    }
    match SyscallNamespace::of(r11) {
        Some(SyscallNamespace::Lv2) => SyscallClassification::Lv2 { number: r11 },
        Some(SyscallNamespace::HleImport) => {
            let (start, _) = SyscallNamespace::HleImport.range();
            SyscallClassification::HleImport {
                index: (r11 - start) as u32,
            }
        }
        Some(SyscallNamespace::CellGovPrivate) => {
            let (start, _) = SyscallNamespace::CellGovPrivate.range();
            let index = (r11 - start) as u32;
            match CellGovPrivateSyscall::from_index(index) {
                Some(variant) => SyscallClassification::CellGovPrivate(variant),
                None => SyscallClassification::Unknown { r11 },
            }
        }
        None => SyscallClassification::Unknown { r11 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_lv2_syscall() {
        assert_eq!(classify(0, 22), SyscallClassification::Lv2 { number: 22 },);
    }

    #[test]
    fn classify_hle_import_decodes_index() {
        assert_eq!(
            classify(0, 0x10005),
            SyscallClassification::HleImport { index: 5 },
        );
    }

    #[test]
    fn classify_cellgov_private_returns_typed_variant() {
        assert_eq!(
            classify(0, 0x80000),
            SyscallClassification::CellGovPrivate(CellGovPrivateSyscall::CallbackReturn),
        );
    }

    #[test]
    fn classify_unknown_private_index_falls_to_unknown() {
        // 0x80001 is in the CellGovPrivate range but no variant
        // is registered at index 1.
        assert_eq!(
            classify(0, 0x80001),
            SyscallClassification::Unknown { r11: 0x80001 },
        );
    }

    #[test]
    fn classify_above_all_namespaces_falls_to_unknown() {
        assert_eq!(
            classify(0, 0x100000),
            SyscallClassification::Unknown { r11: 0x100000 },
        );
    }

    #[test]
    fn classify_hypercall_routes_distinctly_for_lev_1() {
        assert_eq!(
            classify(1, 22),
            SyscallClassification::Hypercall { lev: 1, r11: 22 },
        );
    }

    /// Every non-zero LEV is a hypercall regardless of r11. PS3
    /// usermode never issues these; the runtime rejects them
    /// before LV2 dispatch.
    #[test]
    fn nonzero_lev_always_routes_to_hypercall() {
        for lev in 1..=63u8 {
            assert!(matches!(
                classify(lev, 22),
                SyscallClassification::Hypercall { .. }
            ));
            // Non-zero LEV must not be classified as LV2 even when
            // r11 lands inside the private namespace.
            assert!(matches!(
                classify(lev, 0x80000),
                SyscallClassification::Hypercall { .. }
            ));
        }
    }
}
