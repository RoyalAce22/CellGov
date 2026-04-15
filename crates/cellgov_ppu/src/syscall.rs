//! LV2 syscall number constants and stub classification.
//!
//! Only the syscalls the microtests actually use are listed by name.
//! The SPU thread group family (numbers 170..=192) is treated as a
//! single range stub: the microtest CRT0 calls several operations in
//! that range (create, initialize, start, join, priority-related
//! calls, plus local-store and signal access up to 192), and they
//! are all classified as benign no-op returns during PPU-only runs
//! until real SPU execution takes over.

/// sys_process_exit.
pub const SYS_PROCESS_EXIT: u64 = 22;

/// sys_tty_write.
pub const SYS_TTY_WRITE: u64 = 403;

/// sys_spu_image_open (load SPU ELF from filesystem).
pub const SYS_SPU_IMAGE_OPEN: u64 = 156;

/// First SPU management syscall (SPU thread group lifecycle).
pub const SYS_SPU_THREAD_GROUP_FIRST: u64 = 170;

/// Last SPU management syscall (inclusive). Covers thread group
/// lifecycle (170..=178) plus the SPU thread local-store and signal
/// access ops (179..=192) that PSL1GHT microtests use (e.g. writing
/// an SPU mailbox from the PPU).
pub const SYS_SPU_THREAD_GROUP_LAST: u64 = 192;

/// Classify a syscall as a stub that returns CELL_OK without doing
/// anything. The stub set covers `sys_spu_image_open`, the managed
/// SPU thread group lifecycle range, and TTY write. Returning
/// success without side effects lets the PPU CRT0 advance past its
/// host boundaries so the runtime can observe its final state.
///
/// Returns `Some(0)` when the syscall is a known stub and the
/// execution unit should set `r3 = 0` and advance past the `sc`.
/// Returns `None` when the syscall is not recognized and the unit
/// should fault.
pub fn lv2_stub_return_value(syscall_num: u64) -> Option<u64> {
    match syscall_num {
        SYS_SPU_IMAGE_OPEN => Some(0),
        SYS_TTY_WRITE => Some(0),
        n if (SYS_SPU_THREAD_GROUP_FIRST..=SYS_SPU_THREAD_GROUP_LAST).contains(&n) => Some(0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_stubs_return_zero() {
        assert_eq!(lv2_stub_return_value(SYS_SPU_IMAGE_OPEN), Some(0));
        assert_eq!(lv2_stub_return_value(SYS_TTY_WRITE), Some(0));
    }

    #[test]
    fn spu_thread_group_range_is_stubbed() {
        for n in SYS_SPU_THREAD_GROUP_FIRST..=SYS_SPU_THREAD_GROUP_LAST {
            assert_eq!(
                lv2_stub_return_value(n),
                Some(0),
                "syscall {} not stubbed",
                n
            );
        }
        // Bounds are exclusive outside the range.
        assert_eq!(lv2_stub_return_value(SYS_SPU_THREAD_GROUP_FIRST - 1), None);
        assert_eq!(lv2_stub_return_value(SYS_SPU_THREAD_GROUP_LAST + 1), None);
    }

    #[test]
    fn unknown_syscall_is_not_a_stub() {
        assert_eq!(lv2_stub_return_value(SYS_PROCESS_EXIT), None);
        assert_eq!(lv2_stub_return_value(999), None);
    }
}
