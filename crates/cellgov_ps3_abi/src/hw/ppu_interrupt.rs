//! The PPE interrupts problem-state code can cause: their vector
//! addresses, and the SRR1 and DSISR bits that record their cause.
//!
//! The bit numbers are the ones the ISA prints: bit 0 is the most
//! significant bit of the 64-bit SRR1 and of the 32-bit DSISR.

/// The mask of ISA bit `bit` of a 64-bit register.
pub const fn doubleword_bit(bit: u32) -> u64 {
    1 << (63 - bit)
}

/// The mask of ISA bit `bit` of a 32-bit register.
pub const fn word_bit(bit: u32) -> u32 {
    1 << (31 - bit)
}

/// Data storage interrupt vector.
///
/// [PPC-Book3 p:65 s:5.5] Figure 36 gives each interrupt vector's effective address.
pub const VECTOR_DATA_STORAGE: u64 = 0x300;

/// Data segment interrupt vector.
///
/// [PPC-Book3 p:65 s:5.5] Figure 36.
pub const VECTOR_DATA_SEGMENT: u64 = 0x380;

/// Instruction storage interrupt vector.
///
/// [PPC-Book3 p:65 s:5.5] Figure 36.
pub const VECTOR_INSTRUCTION_STORAGE: u64 = 0x400;

/// Instruction segment interrupt vector.
///
/// [PPC-Book3 p:65 s:5.5] Figure 36.
pub const VECTOR_INSTRUCTION_SEGMENT: u64 = 0x480;

/// Alignment interrupt vector.
///
/// [PPC-Book3 p:65 s:5.5] Figure 36.
pub const VECTOR_ALIGNMENT: u64 = 0x600;

/// Program interrupt vector.
///
/// [PPC-Book3 p:65 s:5.5] Figure 36.
pub const VECTOR_PROGRAM: u64 = 0x700;

/// Floating-point unavailable interrupt vector.
///
/// [PPC-Book3 p:65 s:5.5] Figure 36.
pub const VECTOR_FP_UNAVAILABLE: u64 = 0x800;

/// System call interrupt vector.
///
/// [PPC-Book3 p:65 s:5.5] Figure 36.
pub const VECTOR_SYSTEM_CALL: u64 = 0xC00;

/// VXU unavailable interrupt vector.
///
/// [CBE-Handbook p:258 s:9.5.15] The PPE's VXU unavailable interrupt is at 0xF20.
pub const VECTOR_VXU_UNAVAILABLE: u64 = 0xF20;

/// SRR1 bit 33: an instruction fetch found no translation.
///
/// [PPC-Book3 p:68 s:5.5.5] An instruction storage interrupt sets bit 33 when no translation is found, 35 for a no-execute or guarded fetch, and 36 for a protection refusal.
pub const SRR1_FETCH_NOT_FOUND: u64 = doubleword_bit(33);

/// SRR1 bit 35: a fetch from no-execute or guarded storage.
///
/// [PPC-Book3 p:68 s:5.5.5] See [`SRR1_FETCH_NOT_FOUND`].
pub const SRR1_FETCH_NO_EXECUTE: u64 = doubleword_bit(35);

/// SRR1 bit 36: storage protection refused a fetch.
///
/// [PPC-Book3 p:68 s:5.5.5] See [`SRR1_FETCH_NOT_FOUND`].
pub const SRR1_FETCH_PROTECTION: u64 = doubleword_bit(36);

/// SRR1 bit 43: a floating-point enabled program interrupt.
///
/// [PPC-Book3 p:72 s:5.5.9] A program interrupt sets bit 43 for floating-point enabled, 44 for illegal instruction, 45 for privileged instruction and 46 for trap, and clears bit 47 when SRR0 addresses the instruction that caused it.
pub const SRR1_PROGRAM_FP_ENABLED: u64 = doubleword_bit(43);

/// SRR1 bit 44: an illegal-instruction program interrupt.
///
/// [PPC-Book3 p:72 s:5.5.9] See [`SRR1_PROGRAM_FP_ENABLED`].
pub const SRR1_PROGRAM_ILLEGAL: u64 = doubleword_bit(44);

/// SRR1 bit 45: a privileged-instruction program interrupt.
///
/// [PPC-Book3 p:72 s:5.5.9] See [`SRR1_PROGRAM_FP_ENABLED`].
pub const SRR1_PROGRAM_PRIVILEGED: u64 = doubleword_bit(45);

/// SRR1 bit 46: a trap program interrupt.
///
/// [PPC-Book3 p:72 s:5.5.9] See [`SRR1_PROGRAM_FP_ENABLED`].
pub const SRR1_PROGRAM_TRAP: u64 = doubleword_bit(46);

/// DSISR bit 1: the access found no translation.
///
/// [PPC-Book3 p:67 s:5.5.3] A data storage interrupt sets DSISR bit 1 when no translation is found, bit 4 when storage protection refuses the access, and bit 6 for a store or `dcbz`.
pub const DSISR_NOT_FOUND: u32 = word_bit(1);

/// DSISR bit 4: storage protection refused the access.
///
/// [PPC-Book3 p:67 s:5.5.3] See [`DSISR_NOT_FOUND`].
pub const DSISR_PROTECTION: u32 = word_bit(4);

/// DSISR bit 6: the access was a store or `dcbz`.
///
/// [PPC-Book3 p:67 s:5.5.3] See [`DSISR_NOT_FOUND`].
pub const DSISR_STORE: u32 = word_bit(6);

/// DSISR bit 10: the PPE sets it on every data segment interrupt.
///
/// [CBE-Handbook p:251 s:9.5.4] A data segment interrupt sets DSISR bit 6 on a store or `dcbz`, sets bit 10, and clears the others.
pub const DSISR_SEGMENT: u32 = word_bit(10);

#[cfg(test)]
#[path = "tests/ppu_interrupt_tests.rs"]
mod tests;
