//! sys_spu PS3 ABI: image and thread-group constants.
//!
//! Behaviour (image open, thread-group create / start / join, mailbox
//! write) lives in `cellgov_lv2::host::spu`; this module is data only.

/// The bytes `sys_spu_image_open` scans for the path's NUL. A path with
/// no NUL in them is a path the kernel does not find: `CELL_ENOENT`.
// The console capture of tests/micro/spu_image_segments shows the
// bound: a path with its NUL at index 1023 reaches the name check, and
// paths of 1024, 1025 and 2048 bytes answer CELL_ENOENT whatever their
// names. The console runs Cobra, which hooks the kernel's path lookup;
// CellGov takes these answers as the kernel's.
pub const IMAGE_PATH_MAX: usize = 1024;

/// The longest name in a path that `sys_spu_image_open` accepts; a
/// longer one answers `CELL_ENAMETOOLONG`.
// The console capture of tests/micro/spu_image_segments shows that a
// 255-byte name answers CELL_ENOENT and a 256-byte name answers
// CELL_ENAMETOOLONG.
pub const IMAGE_PATH_NAME_MAX: usize = 255;

/// The bits of an SPU thread's signal configuration: bit 0 puts
/// signal-notification register 1 in OR mode, bit 1 register 2.
/// `sys_spu_thread_set_spu_cfg` refuses a value with any other bit set,
/// as the `spu_signal_mbox_edges` capture shows for 4.
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
// No public document gives this layout; the open PS3 toolchain's
// sys/spu.h names the same three values. The console capture of
// tests/micro/spu_thread_window_slots shows them. Slot 0 of a
// four-thread group gets each slot's local store, its own included,
// through BASE + n * STRIDE. Its signals reach the SPU_Sig_Notify_1 of
// each other slot through that slot's PROBLEM_STATE area.
pub mod thread_window {
    /// The window's first effective address.
    pub const BASE: u64 = 0xF000_0000;
    /// The bytes each thread slot takes.
    pub const STRIDE: u64 = 0x10_0000;
    /// The offset of a slot's problem-state area; its local store is at
    /// offset 0.
    pub const PROBLEM_STATE: u64 = 0x4_0000;
}

/// The effective-address window in which LV2 maps each Raw SPU: Raw SPU
/// `id` takes `STRIDE` bytes from `BASE + id * STRIDE`, its local store
/// first and its problem-state area at `PROBLEM_STATE`.
// No public document gives this layout; the open PS3 toolchain's
// sys/spu.h names the same three values, and its problem-storage
// accessors reach the registers there. The console capture of
// tests/micro/spu_raw_status runs a Raw SPU through them.
pub mod raw_window {
    /// The window's first effective address.
    pub const BASE: u64 = 0xE000_0000;
    /// The bytes each Raw SPU takes.
    pub const STRIDE: u64 = 0x10_0000;
    /// The offset of a Raw SPU's problem-state area; its local store is
    /// at offset 0.
    pub const PROBLEM_STATE: u64 = 0x4_0000;
}

/// The arguments the Raw SPU calls (150-154, 160, 161, 163, 196, 197,
/// 199) take and the values they hand back.
// The console capture of tests/micro/spu_raw_lifecycle shows each one.
pub mod raw_spu {
    /// An interrupt class whose tag, mask and status a process may
    /// reach: class 0, the error interrupts.
    ///
    /// [CBEA p:274 s:21.7.1] class 0 holds the SPU error, DMA command and DMA alignment interrupts.
    pub const CLASS_ERROR: u32 = 0;
    /// The other class a process may reach: class 2, the application
    /// interrupts. Class 1 belongs to the kernel, and a call naming it
    /// answers `CELL_EINVAL`.
    ///
    /// [CBEA p:275 s:21.7.2] class 1 holds the MFC translation and address-compare interrupts.
    /// [CBEA p:276 s:21.7.3] class 2 holds the mailbox, stop-and-signal and tag-group interrupts.
    pub const CLASS_APPLICATION: u32 = 2;
    /// The `SPU_Cfg` bits `sys_raw_spu_set_spu_cfg` keeps; the console
    /// reads back 0 after a write of 4.
    ///
    /// [CBEA p:239 s:16.4] bits 62 and 63 of SPU_Cfg, its two lowest bits, set the two signal-notification modes.
    pub const CFG_MASK: u32 = 0x3;
    /// The class 2 status bit B, the SPU mailbox threshold interrupt.
    /// The console reads it set on a Raw SPU that never ran, and set
    /// again straight after a write that clears every bit.
    ///
    /// [CBEA p:276 s:21.7.3] bit 59, B, is the SPU mailbox threshold interrupt status.
    pub const INT_STAT_MAILBOX_THRESHOLD: u64 = 0x10;
    /// The class 2 status bit S, set when the SPU stops at a
    /// stop-and-signal instruction.
    ///
    /// [CBEA p:276 s:21.7.3] bit 62, S, is the SPU stop-and-signal instruction trap status.
    pub const INT_STAT_STOP_AND_SIGNAL: u64 = 0x2;
    /// The class 2 status bit M, set by a write to the SPU's outbound
    /// interrupt mailbox.
    ///
    /// [CBEA p:276 s:21.7.3] bit 63, M, is the mailbox interrupt status.
    pub const INT_STAT_MAILBOX: u64 = 0x1;
    /// The top byte of every interrupt tag the console hands back.
    pub const INTERRUPT_TAG_KIND: u32 = 0x0A00_0000;
    /// The distance between two interrupt tags created in turn.
    pub const INTERRUPT_TAG_STEP: u32 = 0x100;
}

/// The 16-byte `sys_spu_image` record `sys_spu_thread_initialize`
/// reads: `type`, `entry_point`, `segs`, `nsegs`, each a big-endian
/// 32-bit word.
// The console capture of tests/micro/spu_image_segments shows the user
// record: a record built word by word at these offsets runs. The
// kernel accepts an entry of ENTRY_MAX and answers CELL_EINVAL for
// ENTRY_MAX + 4. No capture shows a kernel record: the retail kernel
// refuses to open an unsigned SPU ELF, so no probe holds one.
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
    /// counts are refused as well. The `spu_group_init_limits` capture
    /// accepts 0x20 segments and refuses 0 and 0x21 with `CELL_EINVAL`;
    /// `spu_image_nsegs_negative` refuses -1 the same way.
    pub const NSEGS_MAX: i32 = 0x20;
}

/// The 24-byte `sys_spu_segment` record: `type`, `ls`, `size` as
/// big-endian 32-bit words, then a 64-bit-aligned source union whose
/// first word is `addr` (copy source) or `value` (fill value), the
/// layout the firmware's liblv2 `sys_spu_image_import` writes. The
/// union is 64-bit wide, so the record is 0x18 bytes even though the
/// three leading words and the copy source total 0x14.
// The console capture of tests/micro/spu_image_segments shows the
// layout. In a table of 0x18-byte records, a COPY record with decoy
// sources at +0x0C and +0x14 loads from the source at ADDR_OFFSET. A
// segment type of 3 answers CELL_EINVAL.
pub mod segment {
    /// Byte length of the record.
    pub const LEN: u32 = 0x18;
    /// Offset of the `type` word.
    pub const TYPE_OFFSET: u32 = 0;
    /// Offset of the LS destination.
    pub const LS_OFFSET: u32 = 4;
    /// Offset of the byte size.
    pub const SIZE_OFFSET: u32 = 8;
    /// Offset of the copy source address, and of the fill value the
    /// console does not apply.
    pub const ADDR_OFFSET: u32 = 0x10;

    /// `SYS_SPU_SEGMENT_TYPE_COPY`: `size` bytes copied from `addr`.
    pub const TYPE_COPY: u32 = 1;
    /// `SYS_SPU_SEGMENT_TYPE_FILL`: `size` bytes that read zero when
    /// the thread runs, whatever the `value` word holds.
    // The console capture of tests/micro/spu_image_segments shows that
    // two FILL ranges read zero: one with 0x11223344 at ADDR_OFFSET, one
    // with 0x77777777 in all three source words. A local store the
    // kernel clears before it loads reads the same, so the console
    // capture cannot tell a zero fill from no write.
    pub const TYPE_FILL: u32 = 2;
    /// `SYS_SPU_SEGMENT_TYPE_INFO`: metadata, never loaded.
    pub const TYPE_INFO: u32 = 4;

    /// Largest `size` an INFO segment may declare. The
    /// `spu_group_init_limits` capture accepts 256 bytes and refuses
    /// 257 with `CELL_EINVAL`.
    pub const INFO_SIZE_MAX: u32 = 256;
    /// Alignment `ls` and `size` of a loadable segment must satisfy.
    /// The kernel applies this flat check; a caller that builds an
    /// image meets a coarser alignment than the kernel enforces.
    pub const LOAD_ALIGN: u32 = 0x10;
}

/// The numbers the SPU event-binding syscalls take: a thread group's
/// event types, a thread's event ports and its queue bindings.
///
/// No public document names these values; they are unestablished. The
/// open PS3 toolchain's `sys/spu.h` names the run and exception group
/// types, the user thread type, and a port number range of 0..63. The
/// system-module group type and the queue-binding count are not in that
/// header, and nothing here derives them. The binding count fixes the
/// `CELL_EAGAIN` ceiling of `sys_spu_thread_bind_queue`.
pub mod event {
    /// `SYS_SPU_THREAD_GROUP_EVENT_RUN`: the group's threads started or
    /// stopped.
    pub const GROUP_RUN: u32 = 1;
    /// `SYS_SPU_THREAD_GROUP_EVENT_EXCEPTION`: a thread of the group took
    /// an exception.
    pub const GROUP_EXCEPTION: u32 = 2;
    /// `SYS_SPU_THREAD_GROUP_EVENT_SYSTEM_MODULE`: a system-module event
    /// for a group created to cooperate with the system.
    pub const GROUP_SYSTEM_MODULE: u32 = 4;
    /// `SYS_SPU_THREAD_EVENT_USER`: the one event type a per-thread port
    /// connect accepts.
    pub const THREAD_USER: u32 = 0x1;
    /// Event ports per SPU thread; a port number `spup` is below it.
    pub const PORT_COUNT: u32 = 64;
    /// Queue bindings per SPU thread.
    pub const QUEUE_BINDING_COUNT: usize = 32;
}

/// The physical SPUs LV2 hands to a process, which `sys_spu_initialize`
/// splits between SPU threads and Raw SPUs. The kernel ignores the
/// caller's `max_usable_spu` and always uses this count; the reference
/// console's `spu_group_limits` capture shows it.
pub mod pool {
    /// The physical SPUs a process may use.
    pub const USABLE_SPUS: u32 = 6;
    /// The most `max_raw_spu` may be: one SPU stays with SPU threads.
    pub const RAW_SPU_MAX: u32 = 5;
}

/// The `type` word of a `sys_spu_thread_group_attribute`, which decides
/// how a group occupies the SPU pool. The reference console's
/// `spu_group_limits` capture shows each type's accounting.
pub mod group_type {
    /// A regular group, which the scheduler context-switches.
    pub const NORMAL: u32 = 0x00;
    /// The group's memory comes from a container the attribute names.
    pub const MEMORY_FROM_CONTAINER: u32 = 0x04;
    /// The group has no scheduler context.
    pub const NON_CONTEXT: u32 = 0x08;
    /// A group without a context that occupies its SPUs while it lives.
    pub const EXCLUSIVE_NON_CONTEXT: u32 = 0x18;
    /// A group that shares one SPU with the system.
    pub const COOPERATE_WITH_SYSTEM: u32 = 0x20;

    /// The types a process without root permission may create; the
    /// kernel answers `CELL_EINVAL` for any other.
    // The capture shows NORMAL, EXCLUSIVE_NON_CONTEXT and
    // COOPERATE_WITH_SYSTEM accepted and 0x01 refused. The
    // tests/micro/spu_pool_edges capture shows both MEMORY_FROM_CONTAINER
    // forms reaching the container lookup. Every probe EBOOT runs with
    // root permission, so no capture shows which types a process without
    // it may create; that split is unestablished.
    pub const USER_TYPES: [u32; 5] = [
        NORMAL,
        MEMORY_FROM_CONTAINER,
        EXCLUSIVE_NON_CONTEXT,
        COOPERATE_WITH_SYSTEM,
        COOPERATE_WITH_SYSTEM | MEMORY_FROM_CONTAINER,
    ];

    /// The further types only a process with root permission may create.
    /// vsh creates 0x20A.
    // The console capture of tests/micro/spu_group_root_types, from a
    // process with root permission, shows 0x002, 0x00A, 0x022 and 0x102
    // accepted, 0x30A refused, and 0x20A held to one thread. The first
    // 0x20A group answers CELL_EBUSY there: the running system already
    // holds what that type takes. The other entries are unprobed.
    pub const ROOT_TYPES: [u32; 17] = [
        0x002,
        0x006,
        0x00A,
        0x102,
        0x106,
        0x10A,
        0x202,
        0x206,
        0x20A,
        0x902,
        0x906,
        0xA02,
        0xA06,
        0xC02,
        0xC06,
        COOPERATE_WITH_SYSTEM | 0x2,
        COOPERATE_WITH_SYSTEM | 0x6,
    ];

    /// A root type with any of these bits set holds exactly one thread.
    pub const SINGLE_THREAD_MASK: u32 = 0x700;

    /// A type with any of these bits set takes no suspend or resume, and
    /// a yield of it does nothing.
    // Unprobed: only root types carry these bits, and the probes created
    // none of them for suspend; the mask follows the oracle's model.
    pub const NO_RUN_CONTROL_MASK: u32 = 0xF00;
}

/// The priorities of an SPU thread group with a scheduler context. A
/// group without one takes any priority at create and reads back 0.
// The console capture of tests/micro/spu_group_priority shows the root
// range: a root process creates and sets 15, and 256 and -1 answer
// CELL_EINVAL. It shows a cooperating group reading back COOPERATING
// whatever it was created with. Probes run with root permission, so the
// capture cannot show the user floor.
pub mod group_priority {
    /// The lowest priority a process without root permission may give.
    pub const MIN_USER: i32 = 16;
    /// The lowest priority a process with root permission may give.
    pub const MIN_ROOT: i32 = 0;
    /// The highest priority.
    pub const MAX: i32 = 255;
    /// The priority a group that shares an SPU with the system reads
    /// back.
    pub const COOPERATING: i32 = 15;
}

/// The 16-byte `sys_spu_thread_group_attribute` record:
/// `nsize`, `name`, `type`, then the `option` union, each a big-endian
/// 32-bit word.
pub mod group_attr {
    /// Offset of the `type` word.
    pub const TYPE_OFFSET: u32 = 8;
    /// Offset of the `option` word, which names the memory container for
    /// a type with `group_type::MEMORY_FROM_CONTAINER` set.
    pub const CONTAINER_OFFSET: u32 = 12;
}

/// The memory an SPU thread group with a scheduler context takes for
/// each of its threads: the save area of one local store. A group that
/// shares an SPU with the system takes one in all.
// The console capture of tests/micro/spu_group_container shows a 1 MiB
// container holding a group of four threads and refusing a fifth thread
// with CELL_ENOMEM, and a destroy returning the memory;
// tests/micro/spu_group_container_coop shows the cooperating group's one.
pub const GROUP_MEMORY_PER_THREAD: u64 = 0x4_0000;

/// `cause` enum returned by `sys_spu_thread_group_join`.
// The console captures of tests/micro/spu_thread_window_slots and
// tests/micro/spu_stop_service_codes show all three, each with the
// join status it carries: GROUP_EXIT with the exit status,
// ALL_THREADS_EXIT with 0, and TERMINATED with the terminate value.
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
/// No public document names these values. The open PS3 toolchain's
/// SPU thread library issues each one, and the reference console
/// serves each one as the service named here.
///
/// [CBEA p:94 s:8.5.2] a stop-and-signal copies its 14-bit code into bits 2 through 15 of SPU_Status.
// The console captures show each service:
// - tests/micro/spu_thread_window_slots: YIELD resumes the thread,
//   GROUP_EXIT ends the group with its status, and THREAD_EXIT ends
//   the thread.
// - tests/micro/spu_stop_service_codes: RECEIVE_EVENT takes an event
//   from a bound queue. TRY_RECEIVE_EVENT answers CELL_EBUSY on an
//   empty queue and CELL_EINVAL on an unbound queue number.
// Each probe's SPU program issues these codes after its build.sh runs.
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
