//! The syscalls no probe sends. The UART calls reach the system
//! controller and the AV manager behind it, and the gamepad call
//! reaches controller hardware, so only a passive capture of what vsh
//! sends may settle what they answer, never a probe.
//!
//! [`admit`](crate::deploy::admit) refuses, before the runner touches
//! the console, a package whose PPU relink holds one of their numbers
//! in a shape a probe uses to send it. In an executable segment that is
//! a `li r11, N`, the form an inlined `cgov_sc` takes, a `li rD, N`
//! that a `mr r11, rD` copies, or a `li rA, N` into an argument register
//! whose next branch is a call, the form that reaches a wrapper the
//! compiler keeps out of line. In a segment that
//! is not executable it is an aligned 32-bit word `N`, a table of
//! numbers.
//!
//! The scan is a screen, not a proof. A number computed at run time, or
//! a table placed in the executable segment, escapes it. It does not
//! read every `li` or raw word in code: probes load the same small
//! constants as payload offsets and sizes. It reads only the relink, not
//! a PPU program the package carries as a sibling.

use std::path::Path;

use cellgov_ps3_abi::format::elf::{
    ELFCLASS64, ELFDATA2MSB, ELF_EI_CLASS, ELF_EI_DATA, ELF_MAGIC, ELF_PHENTSIZE_OFFSET,
    ELF_PHNUM_OFFSET, ELF_PHOFF_OFFSET, PF_X, PHDR_P_FILESZ_OFFSET, PHDR_P_OFFSET_OFFSET, PT_LOAD,
};
use cellgov_ps3_abi::hw::ppc_isa::{
    PPC_BCCTR_XO, PPC_BCLR_XO, PPC_BC_OPCD, PPC_BRANCH_LK, PPC_B_OPCD, PPC_OR_XO, PPC_XL_OPCD,
    PPC_X_OPCD,
};
use cellgov_ps3_abi::lv2::syscall;

use crate::error::RunnerPs3Error;

/// The syscalls no probe sends: `sys_uart_initialize`,
/// `sys_uart_receive`, `sys_uart_send` and `sys_gamepad_ycon_if`.
/// `sys_uart_get_params` reads two sizes and stays sendable.
pub const PASSIVE_ONLY: [u64; 4] = [
    syscall::UART_INITIALIZE,
    syscall::UART_RECEIVE,
    syscall::UART_SEND,
    syscall::GAMEPAD_YCON_IF,
];

/// `addi rD, r0, 0`, which assembles as `li rD, 0`: the opcode and a
/// zero `rA`, under [`LI_MASK`]. The immediate is the low halfword.
const LI: u32 = 0x3800_0000;

/// The opcode and `rA` fields of an `addi`.
const LI_MASK: u32 = 0xFC1F_0000;

/// `r11`, which carries the syscall number into `sc`.
const SYSCALL_NUMBER_REGISTER: u32 = 11;

/// `r3`-`r10`, the argument registers a number reaches an out-of-line
/// `sc` wrapper through.
const ARGUMENT_REGISTERS: std::ops::RangeInclusive<u32> = 3..=10;

/// How far past an argument-register `li` the scan looks for the branch.
const CALL_WINDOW: usize = 16;

/// `or rA, rS, rB` under [`OR_MASK`]; `mr rA, rS` is `or rA, rS, rS`.
const OR: u32 = (PPC_X_OPCD << 26) | (PPC_OR_XO << 1);

/// The opcode, extended opcode and `Rc` fields of an X-form word.
const OR_MASK: u32 = 0xFC00_07FF;

/// `p_flags` within an ELF64 program header.
const PHDR_P_FLAGS_OFFSET: usize = 4;

fn be_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        bytes.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn be_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn be_u64(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(
        bytes.get(at..at.checked_add(8)?)?.try_into().ok()?,
    ))
}

/// The loadable segments of a big-endian ELF64, as file ranges with
/// whether each is executable; `None` for anything else or a header
/// that runs past the file.
fn loadable_segments(elf: &[u8]) -> Option<Vec<(std::ops::Range<usize>, bool)>> {
    if elf.get(..ELF_MAGIC.len())? != ELF_MAGIC
        || *elf.get(ELF_EI_CLASS)? != ELFCLASS64
        || *elf.get(ELF_EI_DATA)? != ELFDATA2MSB
    {
        return None;
    }
    let phoff = usize::try_from(be_u64(elf, ELF_PHOFF_OFFSET)?).ok()?;
    let phentsize = usize::from(be_u16(elf, ELF_PHENTSIZE_OFFSET)?);
    let phnum = usize::from(be_u16(elf, ELF_PHNUM_OFFSET)?);
    let mut out = Vec::new();
    for i in 0..phnum {
        let ph = phoff.checked_add(i.checked_mul(phentsize)?)?;
        if be_u32(elf, ph)? != PT_LOAD {
            continue;
        }
        let executable = be_u32(elf, ph.checked_add(PHDR_P_FLAGS_OFFSET)?)? & PF_X != 0;
        let start = usize::try_from(be_u64(elf, ph.checked_add(PHDR_P_OFFSET_OFFSET)?)?).ok()?;
        let len = usize::try_from(be_u64(elf, ph.checked_add(PHDR_P_FILESZ_OFFSET)?)?).ok()?;
        // A segment with no file bytes, such as the bss, may name any
        // offset; the probe relinks place theirs past the end of the file.
        if len == 0 {
            continue;
        }
        let end = start.checked_add(len)?;
        if end > elf.len() {
            return None;
        }
        out.push((start..end, executable));
    }
    Some(out)
}

/// The first passive-only syscall number `elf` holds in either shape the
/// module names, with its file offset.
///
/// # Errors
///
/// [`RunnerPs3Error::UnscannableElf`] when `elf` is not a big-endian
/// ELF64 whose program headers lie inside the file.
pub fn first_passive_only_load(
    elf: &[u8],
    path: &Path,
) -> Result<Option<(u64, u64)>, RunnerPs3Error> {
    let segments = loadable_segments(elf).ok_or_else(|| RunnerPs3Error::UnscannableElf {
        elf: path.to_path_buf(),
    })?;
    for (range, executable) in segments {
        let base = range.start;
        let words: Vec<u32> = elf[range]
            .chunks_exact(4)
            .map(|w| u32::from_be_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        for (i, &word) in words.iter().enumerate() {
            let number = if executable {
                if word & LI_MASK != LI {
                    continue;
                }
                let rd = (word >> 21) & 0x1F;
                let rest = &words[i + 1..];
                let feeds_sc = rd == SYSCALL_NUMBER_REGISTER
                    || copies_into_r11(rest, rd)
                    || (ARGUMENT_REGISTERS.contains(&rd) && next_branch_is_a_call(rest));
                if !feeds_sc {
                    continue;
                }
                u64::from(word & 0xFFFF)
            } else {
                u64::from(word)
            };
            if PASSIVE_ONLY.contains(&number) {
                return Ok(Some((number, (base + 4 * i) as u64)));
            }
        }
    }
    Ok(None)
}

/// Whether the first branch among `code` links, within
/// [`CALL_WINDOW`] instructions: `bl`, `bcl`, `bclrl` or `bcctrl`.
fn next_branch_is_a_call(code: &[u32]) -> bool {
    code.iter()
        .take(CALL_WINDOW)
        .find(|&&w| is_branch(w))
        .is_some_and(|&w| w & PPC_BRANCH_LK != 0)
}

/// Whether a `mr r11, rs` follows within [`CALL_WINDOW`] instructions.
fn copies_into_r11(code: &[u32], rs: u32) -> bool {
    code.iter().take(CALL_WINDOW).any(|&w| {
        w & OR_MASK == OR
            && (w >> 21) & 0x1F == rs
            && (w >> 16) & 0x1F == SYSCALL_NUMBER_REGISTER
            && (w >> 11) & 0x1F == rs
    })
}

fn is_branch(word: u32) -> bool {
    match word >> 26 {
        PPC_BC_OPCD | PPC_B_OPCD => true,
        PPC_XL_OPCD => matches!((word >> 1) & 0x3FF, PPC_BCLR_XO | PPC_BCCTR_XO),
        _ => false,
    }
}

#[cfg(test)]
#[path = "tests/passive_tests.rs"]
mod tests;
