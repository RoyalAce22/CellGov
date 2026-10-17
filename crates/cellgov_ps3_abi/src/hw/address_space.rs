//! PS3 LV2 user-process virtual address-space layout.
//!
//! The regions below are the ones CellGov's boot backs. Where a
//! region is CellGov's own layout, its doc says so and gives what the
//! reference console's captures show beside it. The captures are
//! `lv2_address_space`, `lv2_process_layout` and `lv2_rsx_answers`.

/// Base guest virtual address of CellGov's primary-stack region. The
/// stack itself starts one page higher, at [`PS3_PRIMARY_STACK_START`].
/// On the reference console the page at this base is unmapped
/// (`lv2_process_layout`).
pub const PS3_PRIMARY_STACK_BASE: u64 = 0xD000_0000;

/// Size in bytes of CellGov's primary-stack region: room for the
/// largest stack a `sys_proc_param` (PROC_PARAM.primary_stacksize) can
/// get, starting at [`PS3_PRIMARY_STACK_START`]. A console maps only the
/// stack the process gets.
pub const PS3_PRIMARY_STACK_SIZE: usize = 0x0010_1000;

/// Lowest address of the primary thread's stack, whatever its size.
///
/// The reference console's `sys_ppu_thread_get_stack_information`
/// reads this base for every declared size, from 0x2000 to 1 MiB
/// (`tests/micro/lv2_process_stack_*`, `tests/micro/lv2_process_queries`).
pub const PS3_PRIMARY_STACK_START: u64 = 0xD000_1000;

/// Bytes between the top of the primary stack and the primary thread's
/// r1 at entry.
///
/// On the reference console r1 in `main` is 0x3B0 below the top for
/// every declared size. The probe's crt0 takes a 0x80-byte frame and its
/// `main` a 0x100-byte one, so the entry r1 sits 0x230 below the top.
///
/// The reserve holds at least the minimum frame the entry function
/// writes into: it stores CR at 8(r1) and LR at 16(r1), linkage slots
/// its caller is required to have provided.
///
/// [CBE-Handbook p:396 s:14.3.1.3] The loader hands the entry point an R1 that
/// is quadword-aligned and already points at a reserved initial frame carrying a
/// null back chain, so the top of the stack is never itself the SP.
pub const PS3_PRIMARY_STACK_ENTRY_RESERVE: u64 = 0x230;

/// Base of CellGov's child-thread stack region, which sits immediately
/// above its primary-stack region. CellGov's allocator bumps through it
/// and reuses no stack.
///
/// The reference console places them differently (`lv2_address_space`).
/// Its process has the default 64 KiB primary stack. Each of four
/// threads, created and joined in turn, gets its stack at the same
/// address, 0x2000 above the primary stack's end. A joined thread's
/// stack is reused.
pub const PS3_CHILD_STACKS_BASE: u64 = 0xD010_1000;

/// Size in bytes of CellGov's child-thread stack region.
pub const PS3_CHILD_STACKS_SIZE: usize = 0x00EF_F000;

/// Smallest stack frame a PPE 64-bit callee may be handed: the
/// 48-byte fixed header plus the 64-byte minimum parameter save area.
///
/// [CBE-Handbook p:398 s:14.3] The PPE 64-bit standard stack frame holds the
/// back chain at R1+0 and the rest of the fixed slots below R1+48, above which
/// the parameter save area is at least 64 bytes.
pub const PS3_ABI_MIN_STACK_FRAME: u64 = 0x70;

const _: () = assert!(PS3_PRIMARY_STACK_ENTRY_RESERVE >= PS3_ABI_MIN_STACK_FRAME);
// Quadword-aligned SP: the ABI requires it and stack walkers reject an
// r1 that is not a multiple of 16. The start and every adopted size are
// page multiples, so the entry r1 is aligned when the reserve is.
const _: () = assert!(PS3_PRIMARY_STACK_ENTRY_RESERVE.is_multiple_of(0x10));
const _: () = assert!(PS3_PRIMARY_STACK_START.is_multiple_of(0x1000));

/// Smallest `primary_stacksize` the kernel adopts.
///
/// The reference console gives a declared 0x800 or 0xC00 the
/// [`PS3_PRIMARY_STACK_SIZE_DEFAULT`] and a declared 0x1800 0x2000
/// (`tests/micro/lv2_process_stack_*`). The 0x10..0x70 values some
/// tables read as size codes are declarations below this, and get the
/// default too.
pub const PS3_PRIMARY_STACK_SIZE_MIN: u32 = 0x1000;

/// Largest `primary_stacksize` the kernel adopts.
///
/// The reference console gives a declared 0x100000 a 1 MiB stack and
/// a declared 0x101000 the [`PS3_PRIMARY_STACK_SIZE_DEFAULT`].
pub const PS3_PRIMARY_STACK_SIZE_MAX: u32 = 0x10_0000;

/// Primary stack size when the title declares none, or declares one
/// outside [`PS3_PRIMARY_STACK_SIZE_MIN`]..=[`PS3_PRIMARY_STACK_SIZE_MAX`].
///
/// The reference console gives a title with no `sys_proc_param` a
/// 64 KiB stack (`tests/micro/lv2_process_stack_undeclared`).
pub const PS3_PRIMARY_STACK_SIZE_DEFAULT: u32 = 0x1_0000;

/// Granularity the kernel rounds an adopted primary-stack size up to.
///
/// The reference console gives a declared 0x11800 a 0x12000 stack.
pub const PS3_STACK_SIZE_GRANULARITY: u32 = 0x1000;

// Decoders round an adopted `primary_stacksize` byte count in
// [MIN, MAX] up to the granularity. That round-up can only stay
// inside the window while both bounds are themselves granularity
// multiples, and the resulting stack only fits the backed region while
// the region reaches START + MAX -- otherwise a max-declaring title's
// stack silently runs into the child-stack region above it.
const _: () = assert!(PS3_PRIMARY_STACK_SIZE_MIN <= PS3_PRIMARY_STACK_SIZE_MAX);
const _: () = assert!(PS3_PRIMARY_STACK_SIZE_MIN.is_multiple_of(PS3_STACK_SIZE_GRANULARITY));
const _: () = assert!(PS3_PRIMARY_STACK_SIZE_MAX.is_multiple_of(PS3_STACK_SIZE_GRANULARITY));
const _: () = assert!(
    PS3_PRIMARY_STACK_SIZE_MIN <= PS3_PRIMARY_STACK_SIZE_DEFAULT
        && PS3_PRIMARY_STACK_SIZE_DEFAULT <= PS3_PRIMARY_STACK_SIZE_MAX
);
const _: () = assert!(
    PS3_PRIMARY_STACK_START + PS3_PRIMARY_STACK_SIZE_MAX as u64
        <= PS3_PRIMARY_STACK_BASE + PS3_PRIMARY_STACK_SIZE as u64
);
const _: () = assert!(PS3_PRIMARY_STACK_START >= PS3_PRIMARY_STACK_BASE);

/// Base of CellGov's iomap region, a backed region that
/// `sys_rsx_context_iomap` (672) maps into.
///
/// It is CellGov's own layout. On the reference console the IO window
/// libgcm reports is the game's own host buffer, allocated with
/// `sys_memory_allocate` (`0x5000_0000` in `lv2_rsx_answers`), and a
/// game's raw 672 is refused. A 64 KiB page at this base is mapped
/// before any RSX call (`lv2_process_layout`, `lv2_address_space`); what
/// it holds is not captured.
pub const PS3_RSX_IOMAP_BASE: u64 = 0x4000_0000;

/// Size of CellGov's iomap region: a CellGov default, chosen to fit the
/// first `sys_rsx_context_iomap` size seen from a retail title. No
/// console capture fixes it.
pub const PS3_RSX_IOMAP_SIZE: usize = 0x0550_0000;

/// Base of CellGov's RSX region. Reads return zero and writes fault,
/// unless the title's manifest opts into the RSX mirror, which makes it
/// read-write.
///
/// The reference console maps RSX local memory here once a context is
/// allocated: `0xEA0_0000` bytes in 1 MiB pages, read-write, and a word
/// written there reads back (`lv2_address_space`). Before that nothing
/// maps here (`lv2_process_layout`).
pub const PS3_RSX_BASE: u64 = 0xC000_0000;

/// Size in bytes of CellGov's RSX region. It holds the console's local
/// memory with room to spare.
pub const PS3_RSX_SIZE: usize = 0x1000_0000;

/// Base of CellGov's SPU region, with the same read-zero / write-fault
/// model as [`PS3_RSX_BASE`].
///
/// On the reference console the region holds two kinds of window. A
/// raw SPU's windows sit at this base plus `id * 0x10_0000`
/// (`spu_raw_window_modes`). SPU thread windows start at `0xF000_0000`
/// (`spu_thread_window_slots`, `spu_ls_alias`).
pub const PS3_SPU_RESERVED_BASE: u64 = 0xE000_0000;

/// Size in bytes of CellGov's SPU region, which runs to the end of the
/// 32-bit address space.
pub const PS3_SPU_RESERVED_SIZE: usize = 0x2000_0000;

/// Lowest address of a game's image. The reference console maps
/// nothing at 0, 0x8000 or 0xF000, and maps the image's first page here
/// (`lv2_address_space`).
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
