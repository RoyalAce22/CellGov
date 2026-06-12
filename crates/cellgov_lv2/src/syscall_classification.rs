//! LEV-aware dispatch-hint classifier for `sc` yields.
//!
//! Composes the pure namespace partition from
//! `cellgov_ps3_abi::syscall_namespace` with the hypercall guard so
//! the runtime can route an `sc` to LV2, an unresolved-import
//! trampoline, or a fault path off a single typed value.

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
    /// Routes to the unresolved-import diagnostic dispatch.
    UnresolvedImport {
        /// 0-based offset inside the `UnresolvedImport` namespace.
        /// Currently always 0; the NID itself rides in r4 at
        /// dispatch time.
        index: u32,
    },
    // [PPC-Book3 p:73 s:5.5.13 System Call Interrupt] sc with LEV=1 in
    // problem state should be treated as a programming error (hypervisor
    // call from unprivileged context is not permitted).
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
        Some(SyscallNamespace::UnresolvedImport) => {
            let (start, _) = SyscallNamespace::UnresolvedImport.range();
            SyscallClassification::UnresolvedImport {
                index: (r11 - start) as u32,
            }
        }
        None => SyscallClassification::Unknown { r11 },
    }
}

#[cfg(test)]
#[path = "tests/syscall_classification_tests.rs"]
mod tests;
