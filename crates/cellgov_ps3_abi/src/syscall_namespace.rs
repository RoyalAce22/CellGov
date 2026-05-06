//! Single source of truth for the `r11` syscall-number namespace.
//!
//! Every `sc 0` instruction CellGov emits or classifies sources its
//! syscall number from r11. Three disjoint, contiguous ranges share
//! that word:
//!
//! - **`Lv2`** -- real PS3 LV2 syscalls. RPCS3's table tops out
//!   around 1023; the namespace reserves the low 64K (`0..0x10000`).
//! - **`HleImport`** -- CellGov-emitted HLE import trampolines (one
//!   per PS3 PRX import). r11 = `HleImport.encode(hle_index)`. The
//!   runtime classifies the call as a NID lookup.
//! - **`CellGovPrivate`** -- runtime-installed trampolines that fire
//!   CellGov-private control surfaces (callback return today;
//!   future flip-handler return, SPURS exception return, etc.).
//!
//! All emitters go through [`SyscallNamespace::encode`] (proof at
//! const-eval time when the index is statically known) or
//! [`SyscallNamespace::try_encode`] (when the index grows at
//! runtime, e.g. the HLE import binder). Pure namespace facts
//! ([`SyscallNamespace::of`] / [`SyscallNamespace::decode`]) live
//! here; the LEV-aware dispatch-hint classifier lives in
//! `cellgov_lv2::syscall_classification` because it produces a
//! routing decision rather than an ABI fact.
//!
//! Adding a new private syscall becomes "register an index in
//! [`CellGovPrivateSyscall`]" rather than picking a free number,
//! editing two crates, and hoping the bit pattern does not collide.

use crate::syscall;

/// Half-open ranges in the syscall-number namespace.
///
/// Every encoder and the classifier route through this enum. Adding
/// a namespace requires extending the [`Self::range`] and
/// [`Self::of`] match arms together; the const-asserting blocks at
/// the bottom of this module pin disjointness AND contiguity at
/// compile time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyscallNamespace {
    /// Real PS3 LV2 syscalls (`0..0x10000`).
    Lv2,
    /// CellGov-emitted HLE import trampolines (`0x10000..0x80000`).
    /// One number per imported function; the index space gives
    /// 0x70000 (~458K) bindings, well above any plausible title.
    HleImport,
    /// CellGov-private control trampolines (`0x80000..0x100000`).
    /// Indexed by [`CellGovPrivateSyscall`].
    CellGovPrivate,
}

impl SyscallNamespace {
    /// Half-open `[start, end)` range of syscall numbers in this
    /// namespace.
    #[inline]
    pub const fn range(self) -> (u64, u64) {
        match self {
            Self::Lv2 => (0, 0x10000),
            Self::HleImport => (0x10000, 0x80000),
            Self::CellGovPrivate => (0x80000, 0x100000),
        }
    }

    /// Combine `self` with `index` into a syscall number.
    ///
    /// # Panics
    /// In debug builds, panics if `start + index` does not fit in
    /// the namespace's range. In const context this is a compile-
    /// time error. Release builds skip the assert; the call sites
    /// that grow `index` at runtime (the HLE import binder) use
    /// [`Self::try_encode`] to surface overflow as `None` instead
    /// of crashing the emulator.
    #[inline]
    pub const fn encode(self, index: u32) -> u64 {
        let (start, end) = self.range();
        let value = start + index as u64;
        debug_assert!(value < end, "syscall index out of range for namespace");
        value
    }

    /// Fallible variant of [`Self::encode`] for runtime callers
    /// whose `index` grows from a count (one per PRX import in the
    /// HLE binder's case). Returns `None` when the produced syscall
    /// number would land outside the namespace's `[start, end)`
    /// range, including the wrap-past-end case if a future `range()`
    /// edit ever leaves the upper bound below the index reach.
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
    /// `None` means the value falls outside every declared range
    /// (above [`Self::CellGovPrivate`]). Runtime callers folding
    /// in the LEV check use `cellgov_lv2::syscall_classification::classify`,
    /// which composes this function with the hypercall guard.
    #[inline]
    pub const fn of(syscall_num: u64) -> Option<SyscallNamespace> {
        let (_lv2_lo, lv2_hi) = Self::Lv2.range();
        let (_hle_lo, hle_hi) = Self::HleImport.range();
        let (_priv_lo, priv_hi) = Self::CellGovPrivate.range();
        if syscall_num < lv2_hi {
            Some(Self::Lv2)
        } else if syscall_num < hle_hi {
            Some(Self::HleImport)
        } else if syscall_num < priv_hi {
            Some(Self::CellGovPrivate)
        } else {
            None
        }
    }

    /// Decompose a syscall number into `(namespace, index)`.
    ///
    /// `index` is `syscall_num - namespace.range().0`. Returns
    /// `None` for syscall numbers outside every declared range.
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

/// Indexed catalog of CellGov-private syscalls.
///
/// Each variant maps to a unique index in
/// [`SyscallNamespace::CellGovPrivate`]. Adding a private syscall
/// (vblank-handler return, SPURS exception return, etc.) means
/// **appending** a variant; the encoded number is then
/// `SyscallNamespace::CellGovPrivate.encode(<variant> as u32)`.
///
/// # Invariant: never renumber existing variants
/// The discriminant of each variant is wire-visible -- it lands in
/// guest memory as the lo half of `lis r11; ori r11` and is
/// captured in trace fixtures. Renumbering an existing variant
/// changes the encoded bytes and breaks any consumer keyed off the
/// raw number. New variants only ever append.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[repr(u32)]
pub enum CellGovPrivateSyscall {
    /// Worker-callback return trampoline. Fires from a PPU worker
    /// thread after a `blr` lands on the runtime-installed
    /// trampoline body.
    CallbackReturn = 0,
}

impl CellGovPrivateSyscall {
    /// Encoded syscall number for this variant.
    #[inline]
    pub const fn encode(self) -> u64 {
        SyscallNamespace::CellGovPrivate.encode(self as u32)
    }

    /// Recover the variant from a per-namespace `index`. `None` for
    /// indices not yet registered. Used by the LEV-aware classifier
    /// in `cellgov_lv2::syscall_classification` to surface a typed
    /// variant rather than a raw number.
    ///
    /// # Producer / consumer ordering
    /// Adding a variant requires an audit of any code emitting
    /// `CellGovPrivate` trampolines to ensure no emitter precedes
    /// the variant registration. The match arms here make a missing
    /// consumer-side variant a compile error when the classifier is
    /// rebuilt, but a producer (some future flip-handler OPD
    /// installer in a different crate) could still emit the new
    /// syscall number from a cross-crate call site before the
    /// variant exists. Such an emission would route through the
    /// classifier's `Unknown` arm silently. Land the variant + every
    /// emitter in the same change to avoid the gap.
    #[inline]
    pub const fn from_index(index: u32) -> Option<Self> {
        match index {
            0 => Some(Self::CallbackReturn),
            _ => None,
        }
    }
}

// Compile-time disjointness AND contiguity check. Disjointness
// alone would let a future edit leave a gap (`HleImport` at
// 0x10000..0x70000, `CellGovPrivate` at 0x80000..0x100000) where
// `of()` returns None for the 0x70000..0x80000 range -- a silent
// classifier hole. Contiguity asserts the chosen design: every
// r11 value below 0x100000 lands in exactly one namespace.
const _: () = {
    let (lv2_lo, lv2_hi) = SyscallNamespace::Lv2.range();
    let (hle_lo, hle_hi) = SyscallNamespace::HleImport.range();
    let (priv_lo, _) = SyscallNamespace::CellGovPrivate.range();
    assert!(lv2_lo == 0, "Lv2 namespace must start at 0");
    assert!(
        lv2_hi == hle_lo,
        "Lv2 and HleImport must be contiguous (no gap)",
    );
    assert!(
        hle_hi == priv_lo,
        "HleImport and CellGovPrivate must be contiguous (no gap)",
    );
};

// Compile-time proof that every LV2 syscall constant in
// `crate::syscall` fits the `Lv2` namespace. A future syscall
// constant added at, say, `0x10000+` would compile cleanly without
// this block, then silently route as an HLE import at runtime.
// Adding a new constant in `crate::syscall` requires extending this
// table -- the friction is the point.
const LV2_SYSCALL_CATALOG: &[u64] = &[
    syscall::PROCESS_GETPID,
    syscall::PROCESS_GET_NUMBER_OF_OBJECT,
    syscall::PROCESS_GETPPID,
    syscall::PROCESS_EXIT,
    syscall::PROCESS_GET_SDK_VERSION,
    syscall::PROCESS_GET_PARAMSFO,
    syscall::PROCESS_GET_PPU_GUID,
    syscall::TIMER_CREATE,
    syscall::TIMER_DESTROY,
    syscall::RWLOCK_CREATE,
    syscall::RWLOCK_DESTROY,
    syscall::EVENT_PORT_CREATE,
    syscall::EVENT_PORT_DESTROY,
    syscall::PPU_THREAD_EXIT,
    syscall::PPU_THREAD_YIELD,
    syscall::PPU_THREAD_JOIN,
    syscall::PPU_THREAD_CREATE,
    syscall::EVENT_FLAG_CREATE,
    syscall::EVENT_FLAG_DESTROY,
    syscall::EVENT_FLAG_WAIT,
    syscall::EVENT_FLAG_TRY_WAIT,
    syscall::EVENT_FLAG_SET,
    syscall::SEMAPHORE_CREATE,
    syscall::SEMAPHORE_DESTROY,
    syscall::SEMAPHORE_WAIT,
    syscall::SEMAPHORE_TRY_WAIT,
    syscall::SEMAPHORE_POST,
    syscall::LWMUTEX_CREATE,
    syscall::LWMUTEX_DESTROY,
    syscall::MUTEX_DESTROY,
    syscall::LWMUTEX_LOCK,
    syscall::LWMUTEX_UNLOCK,
    syscall::LWMUTEX_TRYLOCK,
    syscall::MUTEX_CREATE,
    syscall::MUTEX_LOCK,
    syscall::MUTEX_TRYLOCK,
    syscall::MUTEX_UNLOCK,
    syscall::COND_CREATE,
    syscall::COND_DESTROY,
    syscall::COND_WAIT,
    syscall::COND_SIGNAL,
    syscall::COND_SIGNAL_ALL,
    syscall::COND_SIGNAL_TO,
    syscall::SEMAPHORE_GET_VALUE,
    syscall::EVENT_FLAG_CANCEL,
    syscall::EVENT_FLAG_GET,
    syscall::EVENT_FLAG_CLEAR,
    syscall::EVENT_QUEUE_CREATE,
    syscall::EVENT_QUEUE_DESTROY,
    syscall::EVENT_QUEUE_RECEIVE,
    syscall::EVENT_QUEUE_TRY_RECEIVE,
    syscall::EVENT_PORT_SEND,
    syscall::TIME_GET_TIMEZONE,
    syscall::TIME_GET_CURRENT_TIME,
    syscall::TIME_GET_TIMEBASE_FREQUENCY,
    syscall::SPU_IMAGE_OPEN,
    syscall::SPU_THREAD_GROUP_CREATE,
    syscall::SPU_THREAD_INITIALIZE,
    syscall::SPU_THREAD_GROUP_START,
    syscall::SPU_THREAD_GROUP_TERMINATE,
    syscall::SPU_THREAD_GROUP_JOIN,
    syscall::SPU_THREAD_WRITE_MB,
    syscall::MEMORY_CONTAINER_CREATE,
    syscall::MEMORY_ALLOCATE,
    syscall::MEMORY_FREE,
    syscall::MEMORY_GET_USER_MEMORY_SIZE,
    syscall::TTY_WRITE,
    syscall::FS_OPEN,
    syscall::FS_READ,
    syscall::FS_WRITE,
    syscall::FS_CLOSE,
    syscall::FS_FSTAT,
    syscall::FS_STAT,
    syscall::FS_LSEEK,
    syscall::SYS_RSX_MEMORY_ALLOCATE,
    syscall::SYS_RSX_MEMORY_FREE,
    syscall::SYS_RSX_CONTEXT_ALLOCATE,
    syscall::SYS_RSX_CONTEXT_FREE,
    syscall::SYS_RSX_CONTEXT_ATTRIBUTE,
];

const _: () = {
    let lv2_hi = SyscallNamespace::Lv2.range().1;
    let mut i = 0;
    while i < LV2_SYSCALL_CATALOG.len() {
        assert!(
            LV2_SYSCALL_CATALOG[i] < lv2_hi,
            "LV2 syscall constant escaped the Lv2 namespace; widen the namespace or rehome the constant",
        );
        i += 1;
    }
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin every namespace's range; loud failure if a future edit
    /// shifts a boundary without updating call sites.
    #[test]
    fn namespace_ranges_are_pinned() {
        assert_eq!(SyscallNamespace::Lv2.range(), (0, 0x10000));
        assert_eq!(SyscallNamespace::HleImport.range(), (0x10000, 0x80000));
        assert_eq!(
            SyscallNamespace::CellGovPrivate.range(),
            (0x80000, 0x100000)
        );
    }

    #[test]
    fn namespaces_are_pairwise_disjoint() {
        let all = [
            SyscallNamespace::Lv2,
            SyscallNamespace::HleImport,
            SyscallNamespace::CellGovPrivate,
        ];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                let (a_lo, a_hi) = a.range();
                let (b_lo, b_hi) = b.range();
                let overlap = a_lo.max(b_lo) < a_hi.min(b_hi);
                assert!(
                    !overlap,
                    "namespaces {a:?} ({a_lo:#x}..{a_hi:#x}) and {b:?} ({b_lo:#x}..{b_hi:#x}) overlap",
                );
            }
        }
    }

    /// Boundary round-trips for each namespace at index 0, midpoint,
    /// and max-valid. Replaces the prior 6-pair sample.
    #[test]
    fn encode_decode_round_trips_at_boundaries() {
        let cases = [
            (SyscallNamespace::Lv2, 0u32),
            (SyscallNamespace::Lv2, 0x8000),
            (SyscallNamespace::Lv2, 0xFFFF),
            (SyscallNamespace::HleImport, 0),
            (SyscallNamespace::HleImport, 0x40000),
            (SyscallNamespace::HleImport, 0x6FFFF),
            (SyscallNamespace::CellGovPrivate, 0),
            (SyscallNamespace::CellGovPrivate, 0x40000),
            (SyscallNamespace::CellGovPrivate, 0x7FFFF),
        ];
        for (ns, index) in cases {
            let n = ns.encode(index);
            assert_eq!(SyscallNamespace::decode(n), Some((ns, index)));
            assert_eq!(SyscallNamespace::of(n), Some(ns));
        }
    }

    #[test]
    fn encode_at_max_index_fits_each_namespace() {
        assert_eq!(SyscallNamespace::Lv2.encode(0xFFFF), 0xFFFF);
        assert_eq!(SyscallNamespace::HleImport.encode(0x6FFFF), 0x7FFFF);
        assert_eq!(SyscallNamespace::CellGovPrivate.encode(0x7FFFF), 0xFFFFF);
    }

    #[test]
    fn of_returns_none_above_highest_namespace() {
        assert_eq!(SyscallNamespace::of(0x100000), None);
        assert_eq!(SyscallNamespace::of(u64::MAX), None);
    }

    #[test]
    fn decode_returns_none_above_highest_namespace() {
        assert_eq!(SyscallNamespace::decode(0x100000), None);
        assert_eq!(SyscallNamespace::decode(u64::MAX), None);
    }

    #[test]
    fn boundary_values_classify_correctly() {
        assert_eq!(SyscallNamespace::of(0xFFFF), Some(SyscallNamespace::Lv2));
        assert_eq!(
            SyscallNamespace::of(0x10000),
            Some(SyscallNamespace::HleImport)
        );
        assert_eq!(
            SyscallNamespace::of(0x7FFFF),
            Some(SyscallNamespace::HleImport)
        );
        assert_eq!(
            SyscallNamespace::of(0x80000),
            Some(SyscallNamespace::CellGovPrivate)
        );
        assert_eq!(
            SyscallNamespace::of(0xFFFFF),
            Some(SyscallNamespace::CellGovPrivate)
        );
    }

    #[test]
    fn callback_return_index_zero_in_private_namespace() {
        let n = CellGovPrivateSyscall::CallbackReturn.encode();
        assert_eq!(n, 0x80000);
        assert_eq!(
            SyscallNamespace::of(n),
            Some(SyscallNamespace::CellGovPrivate)
        );
    }

    /// Variant ordering is wire-visible. Renumbering an existing
    /// variant changes the encoded syscall number, which changes
    /// the bytes the trampoline emits and any trace fixture that
    /// records private syscall numbers. Only ever append new
    /// variants.
    #[test]
    fn private_syscall_discriminants_are_pinned() {
        assert_eq!(CellGovPrivateSyscall::CallbackReturn as u32, 0);
        assert_eq!(CellGovPrivateSyscall::CallbackReturn.encode(), 0x80000);
        assert_eq!(
            CellGovPrivateSyscall::from_index(0),
            Some(CellGovPrivateSyscall::CallbackReturn),
        );
        assert_eq!(CellGovPrivateSyscall::from_index(1), None);
    }

    // ---- encode panic boundaries ----

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "syscall index out of range")]
    fn encode_panics_at_lv2_upper_bound() {
        let _ = SyscallNamespace::Lv2.encode(0x10000);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "syscall index out of range")]
    fn encode_panics_at_hle_upper_bound() {
        let _ = SyscallNamespace::HleImport.encode(0x70000);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "syscall index out of range")]
    fn encode_panics_at_cellgov_private_upper_bound() {
        let _ = SyscallNamespace::CellGovPrivate.encode(0x80000);
    }

    // ---- try_encode ----

    #[test]
    fn try_encode_returns_some_within_range() {
        assert_eq!(SyscallNamespace::Lv2.try_encode(0), Some(0));
        assert_eq!(SyscallNamespace::Lv2.try_encode(0xFFFF), Some(0xFFFF));
        assert_eq!(
            SyscallNamespace::HleImport.try_encode(0x6FFFF),
            Some(0x7FFFF),
        );
    }

    #[test]
    fn try_encode_returns_none_at_upper_bound() {
        assert_eq!(SyscallNamespace::Lv2.try_encode(0x10000), None);
        assert_eq!(SyscallNamespace::HleImport.try_encode(0x70000), None);
        assert_eq!(SyscallNamespace::CellGovPrivate.try_encode(0x80000), None);
    }

    #[test]
    fn try_encode_returns_none_when_index_overflows_u32_to_u64() {
        // Even an index that would wrap past `end` (impossible
        // today since end > start + u32::MAX is structurally false,
        // but pin the wrap-defense behavior so a future range edit
        // that flips this can't introduce a silent overflow).
        assert_eq!(SyscallNamespace::Lv2.try_encode(u32::MAX), None);
    }

    // The LEV-aware dispatch-hint classifier (and its tests) live
    // in `cellgov_lv2::syscall_classification` because they
    // produce a routing decision rather than an ABI fact. The pure
    // `of` / `decode` namespace functions stay tested here.
}
