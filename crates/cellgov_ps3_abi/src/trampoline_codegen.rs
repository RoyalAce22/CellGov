//! Single source of truth for PPC64 trampoline-instruction encoding.
//!
//! Every CellGov-emitted trampoline (HLE import binder, callback-
//! return surface, future flip-handler return) materializes a
//! syscall number in r11 with a `lis r11, hi; ori r11, r11, lo`
//! pair and fires `sc 0`. This module owns the byte encoding so the
//! HLE binder and the callback-dispatch trampoline cannot drift
//! independently. Adding a new trampoline use case calls into the
//! same builders.
//!
//! Encodings follow PowerPC Operating Environment Architecture
//! (PPC64 ELFv1):
//!
//! - `lis rD, SIMM` -- Book I §3.3.9. Equivalent to
//!   `addis rD, 0, SIMM`. SIMM is sign-extended into the GPR's
//!   upper 32 bits, so the encoder rejects inputs whose bit 31 is
//!   set: a syscall number `>= 0x8000_0000` would land at
//!   `0xFFFF_FFFF_8xxx_xxxx` after `lis`, not at `0x0000_0000_8xxx_xxxx`.
//! - `ori rA, rS, UIMM` -- opcode 24. Zero-extends UIMM.
//! - `sc LEV` -- Book III §2.3.1. Bit 30 is a mandatory `1`; bit 0
//!   would produce POWER's invalid "fast SVC" form (Book I
//!   Appendix E.11). LEV=0 is the user-mode syscall path; LEV=1 is
//!   the hypervisor hcall path (CBE Handbook §11.1). LEV>1 is
//!   reserved per Book I §2.4.2.
//! - `blr` (branch to LR) -- extended mnemonic for `bclr 20, 0, 0`.
//!   BO=20 has BO bit 2 set, so the form is valid; Book I §2.4.3
//!   marks `bclr` with BO bit 2 cleared as an invalid form.
//!
//! # Naming convention for OPDs
//!
//! `encode_ps3_packed_opd` produces the 8-byte `(code_addr_u32,
//! toc_u32)` shape used by the HLE binder's `Ps3Spec` / `Legacy24`
//! layouts -- this is RPCS3's `vm::alloc(N*8, vm::main)` table
//! convention, NOT the PPC64 ELFv1 OPD layout. True ELFv1 OPDs are
//! 24 bytes `(code_addr_u64, toc_u64, env_u64)` per CBE Handbook
//! §11.8.2. CellGov has no current consumer for the 24-byte form;
//! the packed function is named explicitly so a future ELFv1 use
//! case adds a separate `encode_opd_elfv1` rather than mistaking
//! the existing function for one.

// ---- `sc LEV` field-level constants -----------------------------
//
// The named constants pin which bit of the instruction word each
// piece of `sc` claims, so a future "let me make LEV configurable"
// edit cannot accidentally clear bit 30 (the mandatory `1`). The
// invariants are doc-pinned at the module level above; here they
// are bit-pinned at the struct level.

/// Opcode field of `sc` (bits 0..6 from MSB). Equivalent to
/// `17u32 << 26`.
const SC_OPCODE: u32 = 17 << 26;

/// LEV=0 (user-mode syscall). Per Book III §2.3.1 the LEV field
/// occupies instruction bits 20:26 (7 bits, MSB-numbered); Book I
/// §2.4.2 reserves bits 20:25, leaving bit 26 as the only active
/// LEV bit. Bit 26 from MSB is bit 5 from LSB in the 32-bit word.
const SC_LEV_USER: u32 = 0 << 5;

/// Mandatory `1` at bit 30 from MSB (bit 1 from LSB). Without this,
/// the instruction decodes to POWER's pre-PowerPC "fast SVC" form,
/// which is invalid per Book I Appendix E.11.
const SC_BIT_30_MANDATORY: u32 = 1 << 1;

/// Highest valid `lis` SIMM input before sign-extension contaminates
/// the upper 32 bits of the destination GPR. Inputs above this would
/// require a different materialization sequence
/// (`lis; ori; rldicl` or similar zero-extension).
const LIS_SIGN_SAFE_LIMIT: u32 = 0x8000_0000;

/// PPC64 instruction bytes for `lis r11, hi; ori r11, r11, lo;
/// sc 0` where `(hi, lo)` materializes `syscall_num` in 32 bits.
///
/// Output is 12 bytes big-endian, ready to commit into guest
/// memory verbatim. The trampoline-OPD layout adds an 8-byte
/// `(code_addr, toc)` after the body when used as a callable
/// function pointer; HLE binder layouts append a `blr` after `sc 0`
/// for run-as-import shape via [`encode_blr`].
///
/// The `u32` parameter is a deliberate type wall: `lis`/`ori`
/// materialize at most 32 bits, and CellGov's
/// [`crate::syscall_namespace::SyscallNamespace`] ranges all live
/// below `0x100000`. A future namespace expansion that needs more
/// bits must use a different materialization sequence and pick a
/// new encoder name; silent truncation cannot happen.
///
/// # Panics
/// In debug builds, panics if `syscall_num >= 0x8000_0000`. The
/// `lis` immediate is sign-extended (Book I §3.3.9), so a bit-31-
/// set input would land in r11 as `0xFFFF_FFFF_8xxx_xxxx` instead
/// of `0x0000_0000_8xxx_xxxx`. Release builds skip the assert; the
/// type wall above plus the namespace ranges keep callers safe in
/// practice.
#[inline]
pub const fn encode_lis_ori_sc(syscall_num: u32) -> [u8; 12] {
    debug_assert!(
        syscall_num < LIS_SIGN_SAFE_LIMIT,
        "syscall_num bit 31 set would sign-extend through the upper 32 bits of r11; \
         use a zero-extension materialization sequence instead",
    );
    let hi = (syscall_num >> 16) & 0xFFFF;
    let lo = syscall_num & 0xFFFF;
    let lis: u32 = (15 << 26) | (11 << 21) | hi;
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
///
/// `blr` is the extended mnemonic `bclr 20, 0, 0`. BO=20 sets bit 2
/// of BO, which Book I §2.4.3 requires for a valid `bclr` form;
/// without it the instruction is invalid. The function is parameter-
/// less today because there is no consumer for parameterized `bclr`;
/// a future producer would lift this to `encode_bclr(bo, bi, bh)`
/// with a `debug_assert` on the BO-bit-2 invariant.
#[inline]
pub const fn encode_blr() -> [u8; 4] {
    let raw: u32 = (19 << 26) | (20 << 21) | (16 << 1);
    raw.to_be_bytes()
}

// Compile-time pin: a future tweak to `encode_blr` (e.g. swapping
// to a different XO or BO) trips the build before any test runs.
const _: () = assert!(
    u32::from_be_bytes(encode_blr()) == 0x4E80_0020,
    "blr canonical encoding drifted",
);

/// RPCS3-compatible packed OPD: `(code_addr_u32, toc_u32)` at 8-byte
/// stride. Used by `cellgov_ppu::prx::HleLayout::Ps3Spec` and
/// `Legacy24` to match RPCS3's `vm::alloc(N*8, vm::main)` HLE-table
/// layout. NOT a valid PPC64 ELFv1 OPD; only safe when the call
/// site dereferences via the packed convention.
///
/// True PPC64 ELFv1 OPDs are 24 bytes `(code_addr_u64, toc_u64,
/// env_u64)`; CellGov has no consumer for that shape today. When
/// one appears, add a separate `encode_opd_elfv1`.
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

    /// Wire-format golden for `sc 0` (callback-return / HLE-stub
    /// trampolines). Drift here means a guest binary would observe
    /// different bytes after a recompile.
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

    /// Highest sign-safe input. `lis r11, 0x7FFF` lands at
    /// `0x0000_0000_7FFF_xxxx`; 0x8000 would sign-extend to
    /// `0xFFFF_FFFF_8000_xxxx` and trip the debug_assert above.
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

    /// Pinned: `sc` LSB byte must always be `0x02`. Bit 30 is the
    /// mandatory `1`; without it the instruction decodes to POWER's
    /// invalid "fast SVC" form per Book I Appendix E.11.
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

    /// Sign-extension contract: bit 31 of input is rejected in debug
    /// builds. The type wall (parameter is `u32`, not `u64`) plus
    /// the namespace ranges keep callers safe in release.
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
        // The shape every current trampoline emits (toc=0): the
        // OPD points at code_addr and carries a zero toc because
        // CellGov's HLE bodies don't read the toc.
        assert_eq!(
            encode_ps3_packed_opd(0x0000_FF00, 0),
            [0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00],
        );
    }
}
