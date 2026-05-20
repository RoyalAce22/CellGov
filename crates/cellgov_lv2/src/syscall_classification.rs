//! LEV-aware dispatch-hint classifier for `sc` yields.
//!
//! Composes the pure namespace partition from
//! `cellgov_ps3_abi::syscall_namespace` with the hypercall guard so
//! the runtime can route an `sc` to LV2, an HLE binder, or a fault
//! path off a single typed value.

use cellgov_ps3_abi::syscall_namespace::SyscallNamespace;

/// Dispatch hint produced by [`classify`] for a guest `sc` instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SyscallClassification {
    /// Routes to the LV2 syscall table.
    Lv2 {
        /// LV2 syscall number from `r11`.
        number: u64,
    },
    /// Routes to the HLE import binder.
    HleImport {
        /// 0-based offset inside the `HleImport` namespace.
        index: u32,
    },
    /// Routes to the hypercall fault path; LEV >= 1 cannot originate from PS3 usermode.
    Hypercall {
        /// Privilege level from the `sc` operand.
        lev: u8,
        /// Raw `r11` preserved for diagnostics.
        r11: u64,
    },
    /// Routes to [`crate::Lv2Request::Unsupported`].
    Unknown {
        /// Raw `r11` that did not match any namespace.
        r11: u64,
    },
}

/// Non-zero LEV short-circuits to [`SyscallClassification::Hypercall`]
/// regardless of r11.
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
    fn classify_above_all_namespaces_falls_to_unknown() {
        assert_eq!(
            classify(0, 0x80000),
            SyscallClassification::Unknown { r11: 0x80000 },
        );
    }

    #[test]
    fn classify_hypercall_routes_distinctly_for_lev_1() {
        assert_eq!(
            classify(1, 22),
            SyscallClassification::Hypercall { lev: 1, r11: 22 },
        );
    }

    #[test]
    fn nonzero_lev_always_routes_to_hypercall() {
        for lev in 1..=63u8 {
            assert!(matches!(
                classify(lev, 22),
                SyscallClassification::Hypercall { .. }
            ));
            assert!(matches!(
                classify(lev, 0x80000),
                SyscallClassification::Hypercall { .. }
            ));
        }
    }
}
