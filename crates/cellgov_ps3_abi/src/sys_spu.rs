//! sys_spu PS3 ABI: image and thread-group constants.
//!
//! Behaviour (image open, thread-group create / start / join, mailbox
//! write) lives in `cellgov_lv2::host::spu`; this module is data only.

/// Maximum bytes `sys_spu_image_open` scans for the path NUL terminator.
pub const IMAGE_PATH_MAX: usize = 256;

/// `cause` enum returned by `sys_spu_thread_group_join`.
pub mod group_join_cause {
    /// `SYS_SPU_THREAD_GROUP_JOIN_GROUP_EXIT`: the group exited
    /// because every thread reached `sys_spu_thread_exit`.
    pub const GROUP_EXIT: u32 = 0x0001;
}
