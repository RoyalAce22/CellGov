//! Single source of truth for the `r11` syscall-number namespace.
//!
//! Three disjoint, contiguous ranges share the r11 word:
//!
//! - **`Lv2`** -- real PS3 LV2 syscalls (`0..0x10000`).
//! - **`HleImport`** -- CellGov-emitted HLE import trampolines
//!   (`0x10000..0x80000`), one number per PRX import.
//! - **`CellGovPrivate`** -- runtime-installed control trampolines
//!   (`0x80000..0x100000`).
//!
//! The LEV-aware dispatch-hint classifier lives in
//! `cellgov_lv2::syscall_classification`; this module exposes pure
//! ABI facts only.
//
// [PPC-Book1 p:36 s:2.4.2 System Call Instruction] sc SC-form: bits
// 0:5 opcode 17, LEV at instruction bits 20:26; bits 0:5 of LEV
// (instr 20:25) reserved; LEV>1 reserved for application use.
// [PPC-Book1 p:2 s:1.5.1 Definitions and Notation] bits are numbered
// left to right starting with bit 0 (big-endian: bit 0 is MSB); range
// p:q denotes bits p through q.
// [PPC-Book1 p:3 s:1.5.2 Reserved Fields and Reserved Values]
// reserved fields in instructions are ignored by the processor;
// software must write zero to maximize forward compatibility.
// [PPC-Book3 p:81 s:5.5.13 System Call Interrupt] sc raises a System
// Call interrupt; SRR0=EA of next instruction; SRR1 loaded from MSR
// (with bits 33:36 and 42:47 cleared); vector at EA 0x0000_0000_0000_0C00.
// [PPC-Book3 p:73 s:5.5.13 System Call Interrupt] sc with LEV=1 in
// problem state should be treated as a programming error (hypervisor
// call from unprivileged context is not permitted).

use crate::syscall;

/// Half-open ranges in the syscall-number namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyscallNamespace {
    /// Real LV2 syscalls in `0..0x10000`.
    Lv2,
    /// CellGov-emitted HLE import trampolines in `0x10000..0x80000`.
    HleImport,
    /// Indexed by [`CellGovPrivateSyscall`].
    CellGovPrivate,
}

impl SyscallNamespace {
    /// Half-open `[start, end)` range of syscall numbers.
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
    /// Returns `None` for values above [`Self::CellGovPrivate`].
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

/// Indexed catalog of CellGov-private syscalls.
///
/// # Invariant
/// Discriminants are wire-visible -- they land in guest memory as
/// the lo half of `lis r11; ori r11` and are captured in trace
/// fixtures. Only ever append new variants; never renumber.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[repr(u32)]
pub enum CellGovPrivateSyscall {
    /// Trampoline a guest callback uses to return control to the runtime.
    CallbackReturn = 0,
}

impl CellGovPrivateSyscall {
    /// Encoded syscall number for this variant.
    #[inline]
    pub const fn encode(self) -> u64 {
        SyscallNamespace::CellGovPrivate.encode(self as u32)
    }

    /// Recover the variant from a per-namespace `index`, or `None`
    /// for indices not yet registered.
    ///
    /// # Cross-crate contract
    /// A producer in another crate can emit a `CellGovPrivate`
    /// syscall number whose variant has not been added here; that
    /// emission routes through the classifier's `Unknown` arm
    /// silently. Land the variant and every emitter in the same
    /// change.
    #[inline]
    pub const fn from_index(index: u32) -> Option<Self> {
        match index {
            0 => Some(Self::CallbackReturn),
            _ => None,
        }
    }
}

// Disjointness + contiguity: every r11 below 0x100000 lands in
// exactly one namespace. A gap would let `of()` return None inside
// the reserved range.
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

// Every LV2 syscall constant in `crate::syscall` must fit the
// `Lv2` namespace; a constant at `0x10000+` would otherwise route
// as an HLE import at runtime. New constants extend this table.
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
    syscall::SPU_IMAGE_IMPORT,
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
    syscall::FS_OPENDIR,
    syscall::FS_READDIR,
    syscall::FS_CLOSEDIR,
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
        assert_eq!(SyscallNamespace::Lv2.try_encode(u32::MAX), None);
    }
}
