//! PS3 process-start argument block for the primary PPU thread.
//!
//! Mirrors the args layout RPCS3 builds in `ppu_load_exec`
//! (`rpcs3/Emu/Cell/PPUModule.cpp`): a pointer table carved off the
//! top of the primary stack -- argv pointers, NULL, envp pointers,
//! NULL, padded to an even slot count -- followed by the NUL-
//! terminated strings, each padded to 16 bytes. The primary
//! thread's r1 lands [`ENTRY_FRAME_RESERVE`] below the block, and
//! the entry receives r3=argc, r4=argv, r5=envp, r6=envp count.

/// One u64 pointer slot in the table.
const SLOT: u64 = 8;

/// String storage granule.
const STRING_ALIGN: u64 = 0x10;

/// Gap between the entry r1 and the block base. A callee stores CR
/// at 8(r1), LR at 16(r1), and may spill into the rest of its
/// caller's 0x70-byte minimum frame (header plus parameter save
/// area), so r1 == block base would let the entry function's
/// prologue overwrite the argv pointer table. RPCS3 gets the same
/// gap from `ppu_stack_start_offset` (0x70): `ppu_load_exec`
/// (`rpcs3/Emu/Cell/PPUModule.cpp`) places the block at the stack
/// top and lowers the constructor-seeded
/// `gpr[1] = stack top - ppu_stack_start_offset`
/// (`rpcs3/Emu/Cell/PPUThread.cpp`, `ppu_thread::ppu_thread`) by the
/// block size.
const ENTRY_FRAME_RESERVE: u64 = 0x70;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum GuestArgsError {
    #[error("argv[{index}] contains a NUL byte; guest strings are NUL-terminated")]
    ArgContainsNul { index: usize },
    #[error(
        "args block of {total} bytes plus the 0x70-byte entry frame reserve \
         does not fit the 0x{stack_size:08x}-byte primary stack"
    )]
    BlockTooLarge { total: u64, stack_size: u64 },
}

/// A fully laid-out args block ready to commit at `base`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct GuestArgsBlock {
    /// Guest address of the block (== `argv_addr`).
    pub base: u64,
    /// Primary thread r1: [`ENTRY_FRAME_RESERVE`] below `base` so
    /// the entry function's linkage-area stores cannot reach the
    /// pointer table.
    pub initial_r1: u64,
    /// Block image: pointer table then string storage.
    pub bytes: Vec<u8>,
    /// r3.
    pub argc: u64,
    /// r4: address of the argv pointer array (== `base`).
    pub argv_addr: u64,
    /// r5: address of the envp pointer array. CellGov passes no
    /// environment, so this points at the NULL terminator slot.
    pub envp_addr: u64,
}

/// Lay out `args` below `stack_top`. Caller skips the call when
/// `args` is empty; an empty slice here still produces a valid
/// argc=0 block, which is NOT the no-args boot shape (r3..r6 = 0).
///
/// # Errors
///
/// [`GuestArgsError::ArgContainsNul`] and
/// [`GuestArgsError::BlockTooLarge`] (checked against `stack_size`).
pub(crate) fn build_args_block(
    stack_top: u64,
    stack_size: u64,
    args: &[String],
) -> Result<GuestArgsBlock, GuestArgsError> {
    for (index, arg) in args.iter().enumerate() {
        if arg.as_bytes().contains(&0) {
            return Err(GuestArgsError::ArgContainsNul { index });
        }
    }
    let n = args.len() as u64;
    // argv pointers + argv NULL + envp NULL, padded to an even slot
    // count so string storage starts 16-aligned.
    let slots = (n + 2).next_multiple_of(2);
    let table_size = slots * SLOT;
    let data_size: u64 = args
        .iter()
        .map(|a| (a.len() as u64 + 1).next_multiple_of(STRING_ALIGN))
        .sum();
    let total = table_size + data_size;
    // The reserve is part of the footprint: r1 must stay on the
    // stack, not just the block.
    if total + ENTRY_FRAME_RESERVE >= stack_size {
        return Err(GuestArgsError::BlockTooLarge { total, stack_size });
    }
    let base = stack_top - total;

    let mut bytes = vec![0u8; total as usize];
    let mut string_addr = base + table_size;
    for (i, arg) in args.iter().enumerate() {
        bytes[i * 8..i * 8 + 8].copy_from_slice(&string_addr.to_be_bytes());
        let off = (string_addr - base) as usize;
        bytes[off..off + arg.len()].copy_from_slice(arg.as_bytes());
        string_addr += (arg.len() as u64 + 1).next_multiple_of(STRING_ALIGN);
    }
    debug_assert_eq!(string_addr, base + total, "string storage misaccounted");

    Ok(GuestArgsBlock {
        base,
        initial_r1: base - ENTRY_FRAME_RESERVE,
        bytes,
        argc: n,
        argv_addr: base,
        envp_addr: base + (n + 1) * SLOT,
    })
}

#[cfg(test)]
#[path = "tests/guest_args_tests.rs"]
mod tests;
