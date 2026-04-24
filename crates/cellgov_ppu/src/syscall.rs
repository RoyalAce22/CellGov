//! LV2 syscall numbers and PPU-only stub classification.
//!
//! Owns the numeric LV2 constants the PPU execution unit recognizes
//! and a classifier that picks out syscalls treated as benign no-op
//! returns when no SPU execution unit is attached. Real dispatch
//! lives in the runtime's LV2 host.

/// sys_process_exit.
pub const SYS_PROCESS_EXIT: u64 = 22;

/// sys_tty_write.
pub const SYS_TTY_WRITE: u64 = 403;

/// sys_spu_image_open.
pub const SYS_SPU_IMAGE_OPEN: u64 = 156;

/// First SPU management syscall (thread group lifecycle).
pub const SYS_SPU_THREAD_GROUP_FIRST: u64 = 170;

/// Last SPU management syscall (inclusive: thread group lifecycle
/// plus local-store and signal access ops).
pub const SYS_SPU_THREAD_GROUP_LAST: u64 = 192;

/// Returns `Some(0)` for syscalls the PPU-only path treats as a
/// successful no-op (r3 = 0, advance past `sc`), `None` for numbers
/// the unit should fault on.
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
        assert_eq!(lv2_stub_return_value(SYS_SPU_THREAD_GROUP_FIRST - 1), None);
        assert_eq!(lv2_stub_return_value(SYS_SPU_THREAD_GROUP_LAST + 1), None);
    }

    #[test]
    fn unknown_syscall_is_not_a_stub() {
        assert_eq!(lv2_stub_return_value(SYS_PROCESS_EXIT), None);
        assert_eq!(lv2_stub_return_value(999), None);
    }
}
