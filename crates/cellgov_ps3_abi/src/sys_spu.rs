//! sys_spu PS3 ABI: image and thread-group constants.
//!
//! Behaviour (image open, thread-group create / start / join, mailbox
//! write) lives in `cellgov_lv2::host::spu`; this module is data only.

/// Maximum bytes `sys_spu_image_open` scans for the path NUL terminator.
pub const IMAGE_PATH_MAX: usize = 256;

/// Local-store size in bytes; segment placement is bounded by it.
// [CBE-Handbook p:64 s:3.1.1] each SPE local store is 256 KB.
pub const LS_SIZE: u32 = 0x4_0000;

/// The 16-byte `sys_spu_image` record `sys_spu_thread_initialize`
/// reads: `type`, `entry_point`, `segs`, `nsegs`, each a big-endian
/// 32-bit word. Cross-reference: RPCS3's `sys_spu.h` `sys_spu_image`.
pub mod image {
    /// Byte length of the record.
    pub const LEN: u32 = 16;
    /// Offset of the `type` word.
    pub const TYPE_OFFSET: u32 = 0;
    /// Offset of `entry_point`; for a kernel image it carries the
    /// kernel's image id instead of an LS address.
    pub const ENTRY_OFFSET: u32 = 4;
    /// Offset of the `segs` pointer.
    pub const SEGS_OFFSET: u32 = 8;
    /// Offset of the signed `nsegs` count.
    pub const NSEGS_OFFSET: u32 = 12;

    /// `SYS_SPU_IMAGE_TYPE_USER`: the record describes segments the
    /// caller laid out itself.
    pub const TYPE_USER: u32 = 0;
    /// `SYS_SPU_IMAGE_TYPE_KERNEL`: the record names an image the
    /// kernel holds.
    pub const TYPE_KERNEL: u32 = 1;

    /// Highest `entry_point` a user image may declare.
    /// Cross-reference: RPCS3's `sys_spu.cpp` `sys_spu_thread_initialize`.
    pub const ENTRY_MAX: u32 = 0x3fffc;
    /// Largest `nsegs` a user image may declare; zero and negative
    /// counts are refused as well.
    /// Cross-reference: RPCS3's `sys_spu.cpp` `sys_spu_thread_initialize`.
    pub const NSEGS_MAX: i32 = 0x20;
}

/// The 24-byte `sys_spu_segment` record: `type`, `ls`, `size` as
/// big-endian 32-bit words, then a 64-bit-aligned source union whose
/// first word is `addr` (copy source) or `value` (fill pattern), the
/// layout the firmware's liblv2 `sys_spu_image_import` writes.
/// Cross-reference: RPCS3's `sys_spu.h` `sys_spu_segment` (checked
/// at 0x18 bytes, the `addr`/`pad` union at 0x10).
pub mod segment {
    /// Byte length of the record.
    pub const LEN: u32 = 0x18;
    /// Offset of the `type` word.
    pub const TYPE_OFFSET: u32 = 0;
    /// Offset of the LS destination.
    pub const LS_OFFSET: u32 = 4;
    /// Offset of the byte size.
    pub const SIZE_OFFSET: u32 = 8;
    /// Offset of the copy source address or the fill value.
    pub const ADDR_OFFSET: u32 = 0x10;

    /// `SYS_SPU_SEGMENT_TYPE_COPY`: `size` bytes copied from `addr`.
    pub const TYPE_COPY: u32 = 1;
    /// `SYS_SPU_SEGMENT_TYPE_FILL`: `size` bytes filled with the
    /// 32-bit `value` pattern.
    pub const TYPE_FILL: u32 = 2;
    /// `SYS_SPU_SEGMENT_TYPE_INFO`: metadata, never loaded.
    pub const TYPE_INFO: u32 = 4;

    /// Largest `size` an INFO segment may declare.
    /// Cross-reference: RPCS3's `sys_spu.cpp` `sys_spu_thread_initialize`.
    pub const INFO_SIZE_MAX: u32 = 256;
    /// Alignment `ls` and `size` of a loadable segment must satisfy.
    /// Cross-reference: RPCS3's `sys_spu.cpp` `sys_spu_thread_initialize`.
    pub const LOAD_ALIGN: u32 = 0x10;
}

/// `cause` enum returned by `sys_spu_thread_group_join`.
pub mod group_join_cause {
    /// `SYS_SPU_THREAD_GROUP_JOIN_GROUP_EXIT`: the group exited
    /// because every thread reached `sys_spu_thread_exit`.
    pub const GROUP_EXIT: u32 = 0x0001;
}
