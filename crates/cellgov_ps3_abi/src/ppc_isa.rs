//! PowerPC ISA instruction-encoding constants shared across the
//! workspace.
//!
//! Pinned opcodes, extended opcodes, BO-field bits, and a handful of
//! complete instruction encodings used by tests, the PPU disassembler,
//! and the stack-walker. Architectural reference: PPC-Book1 (PowerPC
//! v2.02 Book I, the user-mode instruction set).
//!
//! Behaviour (decode, disassembly, opcode synthesis) lives in
//! `cellgov_ppu` and its consumers; this module is data only.

/// Full encoding of `blr` (branch to link register). Used as a
/// terminator in synthetic test ELFs and as a marker in stack walks.
// [PPC-Book1 p:25 s:Branch Conditional to Link Register] BO=20, BH=0, lk=0.
pub const PPC_BLR: u32 = 0x4E80_0020;

/// Big-endian byte encoding of [`PPC_BLR`].
pub const PPC_BLR_BYTES: [u8; 4] = [0x4E, 0x80, 0x00, 0x20];

/// Full encoding of `nop` (`ori r0, r0, 0`). Standard PPC nop slot.
pub const PPC_NOP: u32 = 0x6000_0000;

/// Big-endian byte encoding of [`PPC_NOP`].
pub const PPC_NOP_BYTES: [u8; 4] = [0x60, 0x00, 0x00, 0x00];

/// `bl` opcode template with the link bit (LK) set but the
/// displacement field zeroed. Patching tools OR in the signed
/// 26-bit displacement.
// [PPC-Book1 p:24 s:Branch] B-form, AA=0, LK=1.
pub const PPC_BL_OPCODE_LK: u32 = 0x4800_0001;

/// `b` opcode template without the link bit. Patching tools OR in
/// the signed 26-bit displacement.
pub const PPC_B_OPCODE_NO_LK: u32 = 0x4800_0000;

/// Encoding of `addi r3, r3, 1`. Specific instruction useful as a
/// fixed instruction inside synthetic test bodies.
pub const PPC_ADDI_R3_R3_1: u32 = 0x3863_0001;

/// `BO` field bit 2 (the bit that disables the CTR decrement for
/// conditional-branch-to-CTR variants; the bcctr variant requires
/// it set).
// [PPC-Book1 p:25 s:Branch Conditional to Count Register] BO2=0 invalid for bcctr.
pub const PPC_BO_BIT2: u8 = 0b0_0100;

/// Extended opcode (XO) for `bcctr` (Branch Conditional to Count
/// Register). Used inside the `19 << 26` major-opcode group.
// [PPC-Book1 p:25 s:Branch Conditional to Count Register]
pub const PPC_BCCTR_XO: u32 = 528;

/// Extended opcode (XO) for `bclr` (Branch Conditional to Link
/// Register). Used inside the `19 << 26` major-opcode group.
// [PPC-Book1 p:25 s:Branch Conditional to Link Register]
pub const PPC_BCLR_XO: u32 = 16;
