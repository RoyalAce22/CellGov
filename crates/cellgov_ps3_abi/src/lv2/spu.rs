//! sys_spu PS3 ABI: image and thread-group constants.
//!
//! Behaviour (image open, thread-group create / start / join, mailbox
//! write) lives in `cellgov_lv2::host::spu`; this module is data only.

/// Maximum bytes `sys_spu_image_open` scans for the path NUL terminator.
pub const IMAGE_PATH_MAX: usize = 256;

/// The bits of an SPU thread's signal configuration: bit 0 puts
/// signal-notification register 1 in OR mode, bit 1 register 2.
/// `sys_spu_thread_set_spu_cfg` refuses a value with any other bit set,
/// as the spu_signal_notify microtest's reference baselines show.
///
/// [CBEA p:239 s:16.4] each signal-notification register either overwrites its contents or ORs the data written into them.
pub const SPU_CFG_SIGNAL_MODE_BITS: u64 = 0b11;

/// Local-store size in bytes at the width of the kernel's 32-bit image
/// fields; it bounds segment placement.
///
/// [CBE-Handbook p:64 s:3.1.1] each SPE local store is 256 KB.
pub const LS_SIZE: u32 = crate::hw::spu::SPU_LS_SIZE as u32;

/// The effective-address window in which LV2 maps each thread of an SPU
/// thread group: slot `n` takes `STRIDE` bytes from `BASE + n * STRIDE`,
/// its local store first and its problem-state area at `PROBLEM_STATE`.
///
/// No public document gives this layout. The values are the ones the
/// oracle's MFC transfer path decodes (SPUThread.cpp
/// `spu_thread::do_dma_transfer`).
pub mod thread_window {
    /// The window's first effective address.
    pub const BASE: u64 = 0xF000_0000;
    /// The bytes each thread slot takes.
    pub const STRIDE: u64 = 0x10_0000;
    /// The offset of a slot's problem-state area; its local store is at
    /// offset 0.
    pub const PROBLEM_STATE: u64 = 0x4_0000;
}

/// The 16-byte `sys_spu_image` record `sys_spu_thread_initialize`
/// reads: `type`, `entry_point`, `segs`, `nsegs`, each a big-endian
/// 32-bit word.
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

    /// Highest `entry_point` a user image may declare: the last
    /// word-aligned address inside the local store.
    pub const ENTRY_MAX: u32 = 0x3fffc;
    /// Largest `nsegs` a user image may declare; zero and negative
    /// counts are refused as well. The ceiling is unestablished:
    /// nothing here derives 0x20 from anything else.
    pub const NSEGS_MAX: i32 = 0x20;
}

/// The 24-byte `sys_spu_segment` record: `type`, `ls`, `size` as
/// big-endian 32-bit words, then a 64-bit-aligned source union whose
/// first word is `addr` (copy source) or `value` (fill pattern), the
/// layout the firmware's liblv2 `sys_spu_image_import` writes. The
/// union is 64-bit wide, so the record is 0x18 bytes even though the
/// three leading words and the copy source total 0x14.
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

    /// Largest `size` an INFO segment may declare. Unestablished:
    /// INFO carries metadata that is never loaded, and nothing here
    /// says what fixes its ceiling at 256.
    pub const INFO_SIZE_MAX: u32 = 256;
    /// Alignment `ls` and `size` of a loadable segment must satisfy.
    /// The kernel applies this flat check; a caller that builds an
    /// image meets a coarser alignment than the kernel enforces.
    pub const LOAD_ALIGN: u32 = 0x10;
}

/// `cause` enum returned by `sys_spu_thread_group_join`.
pub mod group_join_cause {
    /// `SYS_SPU_THREAD_GROUP_JOIN_GROUP_EXIT`: a thread of the group
    /// called `sys_spu_thread_group_exit`, which ended every thread.
    pub const GROUP_EXIT: u32 = 0x0001;
    /// `SYS_SPU_THREAD_GROUP_JOIN_ALL_THREADS_EXIT`: every thread of the
    /// group reached `sys_spu_thread_exit`.
    pub const ALL_THREADS_EXIT: u32 = 0x0002;
    /// `SYS_SPU_THREAD_GROUP_JOIN_TERMINATED`:
    /// `sys_spu_thread_group_terminate` ended the group.
    pub const TERMINATED: u32 = 0x0004;
}

/// The stop-and-signal codes an SPU thread stops with to ask LV2 for a
/// service. The SPU puts the service's argument in `SPU_WrOutMbox`
/// first.
///
/// No public document names these values; they are unestablished. The
/// open PS3 toolchain's `spu_thread_exit` writes its status to
/// `SPU_WrOutMbox` and stops with [`THREAD_EXIT`](stop_code::THREAD_EXIT),
/// as every SPU program under `tests/micro` shows once its `build.sh`
/// has run. Nothing in the tree witnesses the other values.
///
/// [CBEA p:94 s:8.5.2] a stop-and-signal copies its 14-bit code into bits 2 through 15 of SPU_Status.
pub mod stop_code {
    /// `spu_thread_group_yield`: the thread group gives up its SPUs and
    /// resumes.
    pub const YIELD: u16 = 0x0100;
    /// `sys_spu_thread_group_exit`: end every thread of the group, with
    /// the status in `SPU_WrOutMbox`.
    pub const GROUP_EXIT: u16 = 0x0101;
    /// `sys_spu_thread_exit`: end this thread, with the status in
    /// `SPU_WrOutMbox`.
    pub const THREAD_EXIT: u16 = 0x0102;
    /// `sys_spu_thread_receive_event`: wait on the event queue named in
    /// `SPU_WrOutMbox`.
    pub const RECEIVE_EVENT: u16 = 0x0110;
    /// `sys_spu_thread_tryreceive_event`: poll the event queue named in
    /// `SPU_WrOutMbox`.
    pub const TRY_RECEIVE_EVENT: u16 = 0x0111;
}
