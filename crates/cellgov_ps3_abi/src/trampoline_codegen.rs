//! PPC64 trampoline-instruction encoding for CellGov-emitted stubs.
//!
//! Encodings follow PowerPC Operating Environment Architecture
//! (PPC64 ELFv1):
//!
//! - `lis rD, SIMM`: SIMM is sign-extended into the GPR's upper 32 bits.
//!   See [PPC-Book1 p:51 s:3.3 Add Immediate Shifted].
//! - `ori rA, rS, UIMM`: opcode 24, zero-extends UIMM.
//! - `sc LEV`: bit 30 is a mandatory `1`; without it the instruction
//!   decodes to POWER's invalid "fast SVC" form. LEV=0 is the
//!   user-mode syscall path. See
//!   [PPC-Book1 p:26 s:2.4.2 System Call Instruction] and
//!   [PPC-Book3 p:12 s:2.3.1].
//! - `blr`: extended mnemonic for `bclr 20, 0, 0`; BO bit 2 must be set.
//!   See [PPC-Book1 p:25 s:2.4 Branch Conditional to Link Register].

/// Opcode field of `sc` (bits 0..6 from MSB).
// [PPC-Book1 p:26 s:2.4.2 System Call Instruction] sc primary opcode = 17, SC-form.
const SC_OPCODE: u32 = 17 << 26;

/// LEV=0 (user-mode syscall). The LEV field occupies instruction
/// bits 20:26 (7 bits, MSB-numbered); bits 20:25 are reserved,
/// leaving bit 26 as the only active LEV bit. Bit 26 from MSB is
/// bit 5 from LSB in the 32-bit word.
// [PPC-Book1 p:26 s:2.4.2 System Call Instruction] bits 20:25 reserved; LEV occupies bit 26 (in app programs LEV=0).
const SC_LEV_USER: u32 = 0 << 5;

/// Mandatory `1` at bit 30 from MSB (bit 1 from LSB).
// [PPC-Book1 p:26 s:2.4.2 System Call Instruction] SC-form layout pins a 1 at bit 30.
const SC_BIT_30_MANDATORY: u32 = 1 << 1;

/// Highest `lis` SIMM input before sign-extension contaminates the
/// upper 32 bits of the destination GPR.
// [PPC-Book1 p:51 s:3.3 Add Immediate Shifted] addis result is EXTS(SI||0x0000); SI bit 0 set sign-extends through the upper 32 bits in 64-bit mode.
const LIS_SIGN_SAFE_LIMIT: u32 = 0x8000_0000;

/// PPC64 instruction bytes for `lis r11, hi; ori r11, r11, lo; sc 0`
/// where `(hi, lo)` materializes `syscall_num` in 32 bits.
///
/// Output is 12 bytes big-endian. HLE binder layouts append a `blr`
/// via [`encode_blr`] for run-as-import shape.
///
/// # Panics
///
/// In debug builds, panics if `syscall_num >= 0x8000_0000`: `lis`
/// sign-extends bit 31 into the upper 32 bits of r11.
#[inline]
pub const fn encode_lis_ori_sc(syscall_num: u32) -> [u8; 12] {
    debug_assert!(
        syscall_num < LIS_SIGN_SAFE_LIMIT,
        "syscall_num bit 31 set would sign-extend through the upper 32 bits of r11; \
         use a zero-extension materialization sequence instead",
    );
    let hi = (syscall_num >> 16) & 0xFFFF;
    let lo = syscall_num & 0xFFFF;
    // [PPC-Book1 p:51 s:3.3 Add Immediate Shifted] addis primary opcode = 15, D-form, EXTS(SI||0x0000); lis is `addis Rx,0,value`.
    let lis: u32 = (15 << 26) | (11 << 21) | hi;
    // [PPC-Book1 p:66 s:3.3 OR Immediate] ori primary opcode = 24, D-form, zero-extends UI (no sign-extend).
    let ori: u32 = (24 << 26) | (11 << 21) | (11 << 16) | lo;
    let sc: u32 = SC_OPCODE | SC_LEV_USER | SC_BIT_30_MANDATORY;
    let lis_b = lis.to_be_bytes();
    let ori_b = ori.to_be_bytes();
    let sc_b = sc.to_be_bytes();
    [
        lis_b[0], lis_b[1], lis_b[2], lis_b[3], ori_b[0], ori_b[1], ori_b[2], ori_b[3], sc_b[0],
        sc_b[1], sc_b[2], sc_b[3],
    ]
}

/// PPC `blr` (branch to LR) as 4 bytes big-endian.
#[inline]
pub const fn encode_blr() -> [u8; 4] {
    // [PPC-Book1 p:25 s:2.4 Branch Conditional to Link Register] bclr XL-form: primary opcode 19, extended opcode 16, BO=20 (branch always: BO[0]=BO[2]=1 skips CTR-decrement and CR test).
    // [PPC-Book1 p:24 s:2.4 Branch Instructions] LK bit (instruction bit 31) = 0 selects blr (no LR update); LK=1 would be blrl.
    let raw: u32 = (19 << 26) | (20 << 21) | (16 << 1);
    raw.to_be_bytes()
}

const _: () = assert!(
    u32::from_be_bytes(encode_blr()) == 0x4E80_0020,
    "blr canonical encoding drifted",
);

/// RPCS3-compatible packed OPD: `(code_addr_u32, toc_u32)` at 8-byte
/// stride.
///
/// Matches RPCS3's `vm::alloc(N*8, vm::main)` HLE-table layout used
/// by `cellgov_ppu::prx::HleLayout::Ps3Spec` and `Legacy24`. This is
/// NOT a valid PPC64 ELFv1 OPD (those are 24 bytes
/// `(code_addr_u64, toc_u64, env_u64)`); only safe when the call
/// site dereferences via the packed convention.
#[inline]
pub const fn encode_ps3_packed_opd(code_addr: u32, toc: u32) -> [u8; 8] {
    let code_b = code_addr.to_be_bytes();
    let toc_b = toc.to_be_bytes();
    [
        code_b[0], code_b[1], code_b[2], code_b[3], toc_b[0], toc_b[1], toc_b[2], toc_b[3],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_lis_ori_sc_for_callback_return_syscall_byte_pattern() {
        assert_eq!(
            encode_lis_ori_sc(0x80000),
            [
                0x3D, 0x60, 0x00, 0x08, // lis r11, 8       (hi = 8)
                0x61, 0x6B, 0x00, 0x00, // ori r11, r11, 0  (lo = 0)
                0x44, 0x00, 0x00, 0x02, // sc 0
            ],
        );
    }

    #[test]
    fn encode_lis_ori_sc_for_hle_import_syscall_byte_pattern() {
        // 0x10005 -> hi = 1, lo = 5
        assert_eq!(
            encode_lis_ori_sc(0x10005),
            [
                0x3D, 0x60, 0x00, 0x01, // lis r11, 1
                0x61, 0x6B, 0x00, 0x05, // ori r11, r11, 5
                0x44, 0x00, 0x00, 0x02, // sc 0
            ],
        );
    }

    #[test]
    fn encode_lis_ori_sc_low_half_only_byte_pattern() {
        // 0xFFFF -> hi = 0, lo = 0xFFFF
        assert_eq!(
            encode_lis_ori_sc(0xFFFF),
            [
                0x3D, 0x60, 0x00, 0x00, // lis r11, 0
                0x61, 0x6B, 0xFF, 0xFF, // ori r11, r11, 0xFFFF
                0x44, 0x00, 0x00, 0x02, // sc 0
            ],
        );
    }

    #[test]
    fn encode_lis_ori_sc_high_half_only_byte_pattern() {
        // 0x1234_0000 -> hi = 0x1234, lo = 0
        assert_eq!(
            encode_lis_ori_sc(0x1234_0000),
            [
                0x3D, 0x60, 0x12, 0x34, // lis r11, 0x1234
                0x61, 0x6B, 0x00, 0x00, // ori r11, r11, 0
                0x44, 0x00, 0x00, 0x02, // sc 0
            ],
        );
    }

    #[test]
    fn encode_lis_ori_sc_at_sign_safe_boundary_byte_pattern() {
        assert_eq!(
            encode_lis_ori_sc(0x7FFF_FFFF),
            [
                0x3D, 0x60, 0x7F, 0xFF, // lis r11, 0x7FFF
                0x61, 0x6B, 0xFF, 0xFF, // ori r11, r11, 0xFFFF
                0x44, 0x00, 0x00, 0x02, // sc 0
            ],
        );
    }

    #[test]
    fn sc_byte_pattern_pins_mandatory_bit_30() {
        for syscall_num in [0u32, 1, 0x10000, 0x80000, 0x7FFF_FFFF] {
            let bytes = encode_lis_ori_sc(syscall_num);
            assert_eq!(bytes[8], 0x44, "sc opcode byte for {syscall_num:#x}");
            assert_eq!(bytes[9], 0x00, "sc reserved-zero byte for {syscall_num:#x}");
            assert_eq!(
                bytes[10], 0x00,
                "sc reserved-zero byte for {syscall_num:#x}"
            );
            assert_eq!(bytes[11], 0x02, "sc bit-30 byte for {syscall_num:#x}");
        }
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "would sign-extend")]
    fn encode_lis_ori_sc_rejects_bit_31_set() {
        let _ = encode_lis_ori_sc(0x8000_0000);
    }

    #[test]
    fn encode_blr_canonical_byte_pattern() {
        assert_eq!(encode_blr(), [0x4E, 0x80, 0x00, 0x20]);
    }

    #[test]
    fn encode_ps3_packed_opd_byte_pattern() {
        assert_eq!(
            encode_ps3_packed_opd(0xCAFE_BABE, 0xDEAD_BEEF),
            [0xCA, 0xFE, 0xBA, 0xBE, 0xDE, 0xAD, 0xBE, 0xEF],
        );
    }

    #[test]
    fn encode_ps3_packed_opd_zero_toc_byte_pattern() {
        assert_eq!(
            encode_ps3_packed_opd(0x0000_FF00, 0),
            [0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00],
        );
    }
}
