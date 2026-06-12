//! Single source of truth for the `r11` syscall-number namespace.
//!
//! Two disjoint, contiguous ranges share the r11 word:
//!
//! - **`Lv2`** -- real PS3 LV2 syscalls (`0..0x10000`).
//! - **`UnresolvedImport`** -- CellGov-emitted unresolved-import
//!   pseudo-syscall (`0x10000..0x80000`). Fired by the trampoline
//!   installed in unpatched GOT slots; the NID rides in r4 and the
//!   number itself currently sits at the namespace start
//!   ([`crate::syscall::UNRESOLVED_IMPORT`]).
//!
//! The LEV-aware dispatch-hint classifier lives in
//! `cellgov_lv2::syscall_classification`; this module exposes pure
//! ABI facts only.

use crate::syscall;

/// Half-open ranges in the syscall-number namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyscallNamespace {
    /// Real LV2 syscalls in `0..0x10000`.
    Lv2,
    /// CellGov-emitted unresolved-import pseudo-syscalls in
    /// `0x10000..0x80000`. Currently a single entry sits at the
    /// namespace start; the NID for the offending GOT slot rides in
    /// r4 at dispatch.
    UnresolvedImport,
}

impl SyscallNamespace {
    /// Half-open `[start, end)` range of syscall numbers.
    #[inline]
    pub const fn range(self) -> (u64, u64) {
        match self {
            Self::Lv2 => (0, 0x10000),
            Self::UnresolvedImport => (0x10000, 0x80000),
        }
    }

    /// Combine `self` with `index` into a syscall number.
    ///
    /// # Panics
    /// In debug builds, panics if `start + index` does not fit in
    /// the namespace's range. In const context this is a compile-
    /// time error. Runtime callers whose `index` grows from a count
    /// should use [`Self::try_encode`].
    #[inline]
    pub const fn encode(self, index: u32) -> u64 {
        let (start, end) = self.range();
        let value = start + index as u64;
        debug_assert!(value < end, "syscall index out of range for namespace");
        value
    }

    /// Fallible variant of [`Self::encode`]. Returns `None` when the
    /// produced syscall number would land outside the namespace's
    /// `[start, end)` range, including the wrap-past-end case.
    #[inline]
    pub const fn try_encode(self, index: u32) -> Option<u64> {
        let (start, end) = self.range();
        let value = start.wrapping_add(index as u64);
        if value >= start && value < end {
            Some(value)
        } else {
            None
        }
    }

    /// Classify a raw r11 value into its namespace.
    ///
    /// Returns `None` for values above [`Self::UnresolvedImport`].
    #[inline]
    pub const fn of(syscall_num: u64) -> Option<SyscallNamespace> {
        let (_lv2_lo, lv2_hi) = Self::Lv2.range();
        let (_unres_lo, unres_hi) = Self::UnresolvedImport.range();
        if syscall_num < lv2_hi {
            Some(Self::Lv2)
        } else if syscall_num < unres_hi {
            Some(Self::UnresolvedImport)
        } else {
            None
        }
    }

    /// Decompose a syscall number into `(namespace, index)`, where
    /// `index = syscall_num - namespace.range().0`.
    #[inline]
    pub const fn decode(syscall_num: u64) -> Option<(SyscallNamespace, u32)> {
        match Self::of(syscall_num) {
            Some(ns) => {
                let (start, _) = ns.range();
                Some((ns, (syscall_num - start) as u32))
            }
            None => None,
        }
    }
}

// Disjointness + contiguity: every r11 below 0x80000 lands in
// exactly one namespace. A gap would let `of()` return None inside
// the reserved range.
const _: () = {
    let (lv2_lo, lv2_hi) = SyscallNamespace::Lv2.range();
    let (unres_lo, _unres_hi) = SyscallNamespace::UnresolvedImport.range();
    assert!(lv2_lo == 0, "Lv2 namespace must start at 0");
    assert!(
        lv2_hi == unres_lo,
        "Lv2 and UnresolvedImport must be contiguous (no gap)",
    );
};

// Every LV2 syscall constant in `crate::syscall` must fit the
// `Lv2` namespace; a constant at `0x10000+` would otherwise route
// as an HLE import at runtime. We drive this check from the macro-
// emitted `syscall::ALL_LV2_NUMBERS` (typed-arm set) and the
// unsupported-routed set so a new constant added to either is
// automatically covered.
const _: () = {
    let lv2_hi = SyscallNamespace::Lv2.range().1;
    let mut i = 0;
    while i < syscall::ALL_LV2_NUMBERS.len() {
        assert!(
            syscall::ALL_LV2_NUMBERS[i] < lv2_hi,
            "LV2 syscall constant escaped the Lv2 namespace; widen the namespace or rehome the constant",
        );
        i += 1;
    }
    let mut j = 0;
    while j < syscall::ALL_LV2_UNSUPPORTED_ROUTED_NUMBERS.len() {
        assert!(
            syscall::ALL_LV2_UNSUPPORTED_ROUTED_NUMBERS[j] < lv2_hi,
            "unsupported-routed LV2 syscall escaped the Lv2 namespace",
        );
        j += 1;
    }
};

#[cfg(test)]
#[path = "tests/syscall_namespace_tests.rs"]
mod tests;
