//! Typed dispatch-hint classification for an `sc` yield.
//!
//! The classifier combines the syscall namespace with the LEV gate.

use cellgov_ps3_abi::lv2::namespace::SyscallNamespace;

/// Dispatch hint produced by [`classify`] for a guest `sc` instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SyscallClassification {
    /// Routes to the LV2 syscall table.
    Lv2 {
        /// LV2 syscall number from `r11`.
        number: u64,
    },
    /// Identifies an `r11` value outside the LV2 syscall table.
    NoSuchSyscall {
        /// Raw `r11` at or past the LV2 table boundary.
        r11: u64,
    },
    /// Routes to the unresolved-import diagnostic dispatch.
    UnresolvedImport {
        /// 0-based offset inside the `UnresolvedImport` namespace.
        /// Currently always 0; the NID itself rides in r4 at
        /// dispatch time.
        index: u32,
    },
    /// Routes to the hypercall fault path. Only a caller that passes LEV = 1
    /// by hand reaches it: the PPU stops on `sc` with LEV = 1 and yields no syscall.
    ///
    /// [PPC-Book3 p:12 s:2.3.1] sc with LEV=1 invokes the hypervisor, even
    /// from problem state, and the architecture calls that a programming
    /// error.
    Hypercall {
        /// Privilege level from the `sc` operand.
        lev: u8,
        /// Raw `r11` preserved for diagnostics.
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
        Some(SyscallNamespace::InvalidLv2) | None => SyscallClassification::NoSuchSyscall { r11 },
        Some(SyscallNamespace::UnresolvedImport) => {
            let (start, _) = SyscallNamespace::UnresolvedImport.range();
            SyscallClassification::UnresolvedImport {
                index: (r11 - start) as u32,
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/syscall_classification_tests.rs"]
mod tests;
