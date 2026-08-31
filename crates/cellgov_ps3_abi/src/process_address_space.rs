//! PS3 LV2 user-process virtual address-space layout.

/// Base guest virtual address of the primary thread's stack region.
pub const PS3_PRIMARY_STACK_BASE: u64 = 0xD000_0000;

/// Size in bytes of the primary thread's stack region. 1 MiB
/// matches what real PS3 titles declare in their `sys_proc_param`
/// (PROC_PARAM.primary_stacksize).
pub const PS3_PRIMARY_STACK_SIZE: usize = 0x0010_0000;

/// Sits immediately above the primary stack so child-stack allocator
/// addresses land in real guest memory.
pub const PS3_CHILD_STACKS_BASE: u64 = 0xD010_0000;

/// Size in bytes of the child-thread stacks region.
pub const PS3_CHILD_STACKS_SIZE: usize = 0x00F0_0000;

/// Smallest stack frame a PPE 64-bit callee may be handed: the
/// 48-byte fixed header plus the 64-byte minimum parameter save area.
// [CBE-Handbook p:398 s:14.3] The PPE 64-bit standard stack frame holds the
// back chain at R1+0 and the rest of the fixed slots below R1+48, above which
// the parameter save area is at least 64 bytes.
pub const PS3_ABI_MIN_STACK_FRAME: u64 = 0x70;

/// Initial stack pointer of the primary thread: one minimum stack
/// frame below the top of the primary stack region.
///
/// The reserve is the callee's, not the caller's: the entry function
/// stores CR at 8(r1) and LR at 16(r1) into linkage slots its caller
/// is required to have provided, so anything less than a whole
/// minimum frame puts those stores past the end of the region.
// [CBE-Handbook p:396 s:14.3.1.3] The loader hands the entry point an R1 that
// is quadword-aligned and already points at a reserved initial frame carrying a
// null back chain, so the top of the stack region is never itself the SP.
pub const PS3_PRIMARY_STACK_TOP: u64 =
    PS3_PRIMARY_STACK_BASE + PS3_PRIMARY_STACK_SIZE as u64 - PS3_ABI_MIN_STACK_FRAME;

// The frame the entry function writes into has to lie inside the
// primary stack: at 0x10 of headroom the LR save slot at 16(r1) landed
// on the first byte of the child-stack arena above it.
const _: () = assert!(
    PS3_PRIMARY_STACK_TOP + PS3_ABI_MIN_STACK_FRAME
        <= PS3_PRIMARY_STACK_BASE + PS3_PRIMARY_STACK_SIZE as u64
);
// Quadword-aligned SP: the ABI requires it and stack walkers reject an
// r1 that is not a multiple of 16.
const _: () = assert!(PS3_PRIMARY_STACK_TOP.is_multiple_of(0x10));

/// Smallest primary stack the kernel hands out. A `primary_stacksize`
/// may declare as little as 4 KiB, and the kernel still hands out
/// 64 KiB. The wider floor is an observation, not a stated rule.
pub const PS3_PRIMARY_STACK_SIZE_MIN: u32 = 0x1_0000;

/// Largest primary stack the kernel hands out, and the ceiling on a
/// declared `primary_stacksize`. The loader clamps a larger
/// declaration rather than refusing it -- a loader choice, not a
/// stated rule.
pub const PS3_PRIMARY_STACK_SIZE_MAX: u32 = 0x10_0000;

/// Granularity a clamped primary-stack size is rounded up to.
pub const PS3_STACK_SIZE_GRANULARITY: u32 = 0x1000;

// Decoders clamp a declared `primary_stacksize` into
// [MIN, MAX] and then round up to the granularity. That round-up can
// only stay inside the window while both bounds are themselves
// granularity multiples, and the resulting stack only fits the backed
// region while the region is at least MAX bytes -- otherwise a
// max-declaring title's stack silently runs into the child-stack
// region below it.
const _: () = assert!(PS3_PRIMARY_STACK_SIZE_MIN <= PS3_PRIMARY_STACK_SIZE_MAX);
const _: () = assert!(PS3_PRIMARY_STACK_SIZE_MIN.is_multiple_of(PS3_STACK_SIZE_GRANULARITY));
const _: () = assert!(PS3_PRIMARY_STACK_SIZE_MAX.is_multiple_of(PS3_STACK_SIZE_GRANULARITY));
const _: () = assert!(PS3_PRIMARY_STACK_SIZE as u64 >= PS3_PRIMARY_STACK_SIZE_MAX as u64);

/// Base of the iomap region `sys_rsx_context_iomap` (672) maps into;
/// libgcm asks for an IO window starting here.
pub const PS3_RSX_IOMAP_BASE: u64 = 0x4000_0000;

/// Size of the backed iomap region, captured from the `size` argument
/// of a retail title's first `sys_rsx_context_iomap` call.
pub const PS3_RSX_IOMAP_SIZE: usize = 0x0550_0000;

/// RSX video/local-memory MMIO: reads return zero, writes fault.
pub const PS3_RSX_BASE: u64 = 0xC000_0000;

/// Size in bytes of the RSX MMIO region.
pub const PS3_RSX_SIZE: usize = 0x1000_0000;

/// SPU-shared MMIO: same read-zero / write-fault semantics as [`PS3_RSX_BASE`].
pub const PS3_SPU_RESERVED_BASE: u64 = 0xE000_0000;

/// Size in bytes of the SPU-shared MMIO region.
pub const PS3_SPU_RESERVED_SIZE: usize = 0x2000_0000;

/// Lowest plausible address for PS3 LV2 user text: the trampoline
/// scratch zone (`0..0x10000`) sits below it, user heap and title text
/// above. An LV2 convention with no architectural backing.
pub const PS3_USER_TEXT_FLOOR: u64 = 0x0001_0000;

// The boot-composed regions must tile the address space without
// overlapping. Each bound below is checked against the *end* of the
// region beneath it, not its base: the primary stack starts exactly at
// PS3_RSX_BASE + PS3_RSX_SIZE, so growing the RSX window by one page
// would overlap it with no other signal.
const _: () = assert!(PS3_RSX_IOMAP_BASE + PS3_RSX_IOMAP_SIZE as u64 <= PS3_RSX_BASE);
const _: () = assert!(PS3_RSX_BASE + PS3_RSX_SIZE as u64 <= PS3_PRIMARY_STACK_BASE);
const _: () =
    assert!(PS3_PRIMARY_STACK_BASE + PS3_PRIMARY_STACK_SIZE as u64 <= PS3_CHILD_STACKS_BASE);
const _: () =
    assert!(PS3_CHILD_STACKS_BASE + PS3_CHILD_STACKS_SIZE as u64 <= PS3_SPU_RESERVED_BASE);
// The topmost region must not wrap past the end of the address space.
const _: () = assert!(PS3_SPU_RESERVED_BASE
    .checked_add(PS3_SPU_RESERVED_SIZE as u64)
    .is_some());
// The user-text floor sits below every mapped region, so a diagnostic
// walk rejecting addresses under it cannot reject a real one.
const _: () = assert!(PS3_USER_TEXT_FLOOR < PS3_RSX_IOMAP_BASE);
