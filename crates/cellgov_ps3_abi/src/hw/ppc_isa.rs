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
///
/// [PPC-Book1 p:25 s:Branch Conditional to Link Register] BO=20, BH=0, lk=0.
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
///
/// [PPC-Book1 p:24 s:Branch] B-form, AA=0, LK=1.
pub const PPC_BL_OPCODE_LK: u32 = 0x4800_0001;

/// `b` opcode template without the link bit. Patching tools OR in
/// the signed 26-bit displacement.
pub const PPC_B_OPCODE_NO_LK: u32 = 0x4800_0000;

/// Primary opcode of the B-form `b` / `bl`.
///
/// [PPC-Book1 p:24 s:Branch] B-form, opcode 18.
pub const PPC_B_OPCD: u32 = 18;

/// Primary opcode of the B-form `bc` / `bcl`.
///
/// [PPC-Book1 p:24 s:Branch] B-form, opcode 16.
pub const PPC_BC_OPCD: u32 = 16;

/// Primary opcode of the XL-form group that holds `bclr` and `bcctr`
/// ([`PPC_BCLR_XO`], [`PPC_BCCTR_XO`]).
///
/// [PPC-Book1 p:25 s:Branch Conditional to Link Register] XL-form, opcode 19.
pub const PPC_XL_OPCD: u32 = 19;

/// The link bit (LK) of every branch form: set, the branch writes the
/// return address to the link register.
///
/// [PPC-Book1 p:24 s:Branch] LK is bit 31.
pub const PPC_BRANCH_LK: u32 = 1;

/// Primary opcode of the X-form group that holds `or`.
///
/// [PPC-Book1 p:67 s:Fixed-Point Logical Instructions] OR is X-form, opcode 31.
pub const PPC_X_OPCD: u32 = 31;

/// Extended opcode (XO) of `or`; `mr rA, rS` is `or rA, rS, rS`.
///
/// [PPC-Book1 p:67 s:Fixed-Point Logical Instructions] OR, extended opcode 444.
pub const PPC_OR_XO: u32 = 444;

/// Encoding of `addi r3, r3, 1`. Specific instruction useful as a
/// fixed instruction inside synthetic test bodies.
pub const PPC_ADDI_R3_R3_1: u32 = 0x3863_0001;

/// `BO` field bit 2 (the bit that disables the CTR decrement for
/// conditional-branch-to-CTR variants; the bcctr variant requires
/// it set).
///
/// [PPC-Book1 p:25 s:Branch Conditional to Count Register] BO2=0 invalid for bcctr.
pub const PPC_BO_BIT2: u8 = 0b0_0100;

/// The `BO` bits of a branch that tests neither CTR nor a CR bit.
///
/// [PPC-Book1 p:20 s:2.4.1] `BO` encoding `1z1zz`: branch always.
pub const PPC_BO_ALWAYS: u8 = 0b1_0100;

/// Extended opcode (XO) for `bcctr` (Branch Conditional to Count
/// Register). Used inside the `19 << 26` major-opcode group.
///
/// [PPC-Book1 p:25 s:Branch Conditional to Count Register]
pub const PPC_BCCTR_XO: u32 = 528;

/// Extended opcode (XO) for `bclr` (Branch Conditional to Link
/// Register). Used inside the `19 << 26` major-opcode group.
///
/// [PPC-Book1 p:25 s:Branch Conditional to Link Register]
pub const PPC_BCLR_XO: u32 = 16;

/// Extended opcode (XO) for `isync`. Used inside the `19 << 26`
/// major-opcode group.
///
/// [PPC-Book2 p:22 s:3.3.1] isync XL-form XO 150.
pub const PPC_ISYNC_XO: u32 = 150;

/// Extended opcode (XO) of `sync`, `lwsync` and `ptesync` under primary 31.
///
/// [PPC-Book2 p:26 s:3.3.3] sync 598.
pub const PPC_SYNC_XO: u32 = 598;

/// Extended opcode (XO) of `eieio` under primary 31.
///
/// [PPC-Book2 p:28 s:3.3.3] eieio 854.
pub const PPC_EIEIO_XO: u32 = 854;

/// Extended opcodes under primary 31 of the storage-control hints:
/// dcbst, dcbf, dcbtst, dcbt, dst, dstst, dss and icbi, in ascending
/// order.
///
/// [PPC-Book2 p:21 s:3.2.2] dcbst 54 and dcbf 86.
/// [PPC-Book2 p:19 s:3.2.2] dcbt 278 and dcbtst 246.
/// [PPC-Book2 p:18 s:3.2.1] icbi 982.
/// [AltiVec-PEM p:6-10 s:6.2] dst 342; [AltiVec-PEM p:6-12 s:6.2] dstst 374; [AltiVec-PEM p:6-9 s:6.2] dss 822.
pub const PPC_STORAGE_HINT_XOS: [u32; 8] = [54, 86, 246, 278, 342, 374, 822, 982];

/// High-order fraction bit that distinguishes a QNaN from an SNaN in
/// double format; setting it quiets a propagated SNaN.
///
/// [PPC-Book1 p:91 s:4.3.2] An SNaN becomes a QNaN by setting the high-order fraction bit.
///
/// The reference console agrees (`ppu_granule_nan`): `fadd` and `fmadd`
/// on an SNaN return it with this bit set and its sign and payload kept.
pub const PPC_F64_QUIET_BIT: u64 = 0x0008_0000_0000_0000;

/// The single QNaN every disabled Invalid Operation Exception yields.
///
/// [PPC-Book1 p:91 s:4.3.2] Generated QNaN: sign 0, exponent all 1s, high-order fraction bit 1, rest 0.
///
/// The reference console agrees (`ppu_granule_nan`). Under the
/// process's default FPSCR, the console returns this value for:
///
/// - double precision: 0/0, inf/inf, inf-inf, inf+-inf, 0*inf, the
///   square root of -1, inf*0+1;
/// - single precision: 0/0, inf+-inf, the square root of -1.
pub const PPC_GENERATED_QNAN_F64: u64 = 0x7FF8_0000_0000_0000;
