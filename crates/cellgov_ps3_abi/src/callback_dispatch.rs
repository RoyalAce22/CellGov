//! Worker-thread callback-dispatch primitive ABI constants.
//!
//! CellGov synthesizes a guest-visible re-entry trampoline at boot
//! time so HLE handlers can invoke a title-supplied function pointer
//! on a worker PPU thread and resume only after the worker returns.
//! When the worker's terminal `blr` lands on the trampoline, the
//! trampoline issues a CellGov-private LV2 syscall whose number lives
//! in the high half of the syscall namespace; the runtime classifies
//! it as a callback return rather than a guest-visible LV2 call.
//!
//! # Namespace contract
//!
//! The callback-return syscall is one entry in the
//! [`crate::syscall_namespace::SyscallNamespace::CellGovPrivate`]
//! range. All emitters use [`crate::syscall_namespace::SyscallNamespace::encode`]
//! and the classifier uses [`crate::syscall_namespace::SyscallNamespace::of`];
//! the namespace module is the single source of truth for which
//! r11 values map to which dispatch arm. Adding a new private
//! syscall (vblank-handler return, SPURS exception return) becomes
//! a variant in
//! [`crate::syscall_namespace::CellGovPrivateSyscall`] with no
//! drift between encoder and classifier.
//!
//! # Trampoline layout
//!
//! 32 bytes reserved at [`CALLBACK_RETURN_REGION_BASE`], chosen
//! inside the main user-memory region in the pre-user-heap scratch
//! zone (`0..0x10000`). The PPU's instruction fetch path reads only
//! from the base-0 region, so the trampoline must live there to be
//! executable. The PS3 user heap starts at `0x10000`; CellGov's
//! `mem_alloc_ptr` starts at the same address. Region layout:
//!
//! | Offset | Size | Contents                                            |
//! | ------ | ---- | --------------------------------------------------- |
//! | 0      | 4    | `lis r11, 8`        (high half of `CB_RETURN_SYSCALL`) |
//! | 4      | 4    | `ori r11, r11, 0`   (low half; combined: `r11 = 0x80000`) |
//! | 8      | 4    | `sc 0`              (CellGov-private syscall)       |
//! | 12     | 8    | OPD `(code_addr = 0x0000_FF00, toc = 0)`            |
//! | 20     | 12   | reserved (zero-filled padding)                      |
//!
//! HLE handlers stage [`CALLBACK_RETURN_CODE_ADDR`] (the trampoline
//! body at offset 0) into a worker thread's LR. The worker's
//! terminal `blr` sets `PC = LR` and lands directly on the
//! trampoline body. The [`CALLBACK_RETURN_OPD_ADDR`] OPD slot
//! exists for forward-compat consumers that want the trampoline
//! callable as a PS3 function pointer (e.g. installable as a
//! flip-handler or vblank-handler); the worker-LR path uses
//! the code address directly. Reloading `r11` inside the
//! trampoline removes the dependency on `r11` surviving `blr`
//! from the title's callback (PPC64 ELFv1 marks `r11` volatile).

/// CellGov-private syscall number issued by the callback-return
/// trampoline. Routed to `Lv2Request::CallbackDispatchReturn`.
/// Equivalent to
/// `SyscallNamespace::CellGovPrivate.encode(CallbackReturn as u32)`;
/// kept as a named constant for clarity at call sites and for
/// const-context comparisons in the classifier.
pub const CB_RETURN_SYSCALL: u64 =
    crate::syscall_namespace::CellGovPrivateSyscall::CallbackReturn.encode();

/// Lowest address of the 32-byte callback-return region. Sits in
/// the pre-user-heap scratch zone (`0..0x10000`) inside the main
/// user-memory region; PPU instruction fetch can reach it because
/// the base-0 region is what the fetch path scans. The user heap
/// starts at `0x10000`, so the trampoline cannot collide with any
/// allocator output.
pub const CALLBACK_RETURN_REGION_BASE: u32 = 0x0000_FF00;

/// Reserved region size in bytes. Code (12) + OPD (8) + pad (12).
pub const CALLBACK_RETURN_REGION_SIZE: u32 = 32;

/// Address of the trampoline code (`lis; ori; sc 0`). This is what
/// HLE handlers stage into a worker thread's LR before spawn; the
/// worker's terminal `blr` sets `PC = LR` and lands directly on the
/// trampoline body.
pub const CALLBACK_RETURN_CODE_ADDR: u32 = 0x0000_FF00;

/// Address of the OPD slot. Forward-compat surface for consumers
/// that need a callable function-pointer handle to the trampoline
/// (e.g. registering it as a flip-handler / vblank-handler that
/// title code calls via `bctrl`). Worker LR slots use
/// [`CALLBACK_RETURN_CODE_ADDR`] instead, since `blr` branches to
/// LR directly.
pub const CALLBACK_RETURN_OPD_ADDR: u32 = 0x0000_FF0C;

/// Maximum recursion depth for nested callbacks. Chosen as a
/// debugging cap: real titles recursing past 4-5 callback layers are
/// almost certainly looping, not legitimately nesting.
pub const CALLBACK_DEPTH_CAP: u8 = 8;

/// Big-endian PPC64 instruction bytes for the trampoline body
/// (12 bytes: `lis r11, 8; ori r11, r11, 0; sc 0`). Built via the
/// shared encoder so this and the HLE PRX binder cannot drift on
/// instruction bit patterns.
///
/// `CB_RETURN_SYSCALL` is provably below `0x100000` (its namespace
/// upper bound), so the `as u32` narrowing cannot lose data; the
/// type wall on the encoder forbids accidentally widening this in
/// the future without picking a new materialization sequence.
pub const TRAMPOLINE_CODE_BYTES: [u8; 12] =
    crate::trampoline_codegen::encode_lis_ori_sc(CB_RETURN_SYSCALL as u32);

/// Big-endian RPCS3-packed OPD bytes: code_addr =
/// `CALLBACK_RETURN_CODE_ADDR`, toc = 0. Matches CellGov's HLE
/// thunk layout (8-byte `(u32 code, u32 toc)` pair); not the 24-
/// byte PPC64 ELFv1 OPD shape. The host reads it via
/// `host/ppu_thread.rs::dispatch`.
pub const TRAMPOLINE_OPD_BYTES: [u8; 8] =
    crate::trampoline_codegen::encode_ps3_packed_opd(CALLBACK_RETURN_CODE_ADDR, 0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cb_return_syscall_in_cellgov_private_namespace() {
        use crate::syscall_namespace::SyscallNamespace;
        assert_eq!(
            SyscallNamespace::of(CB_RETURN_SYSCALL),
            Some(SyscallNamespace::CellGovPrivate),
        );
    }

    #[test]
    fn region_layout_is_consistent() {
        assert_eq!(CALLBACK_RETURN_CODE_ADDR, CALLBACK_RETURN_REGION_BASE);
        assert_eq!(
            CALLBACK_RETURN_OPD_ADDR,
            CALLBACK_RETURN_REGION_BASE + TRAMPOLINE_CODE_BYTES.len() as u32,
        );
        assert!(
            (TRAMPOLINE_CODE_BYTES.len() + TRAMPOLINE_OPD_BYTES.len()) as u32
                <= CALLBACK_RETURN_REGION_SIZE,
        );
    }

    #[test]
    fn region_sits_in_pre_user_heap_zone() {
        // Trampoline lives in the 0..0x10000 scratch zone inside
        // the main user-memory region. The user heap starts at
        // 0x10000; the trampoline must end before that to avoid
        // collisions with allocator output.
        let end = CALLBACK_RETURN_REGION_BASE as u64 + CALLBACK_RETURN_REGION_SIZE as u64;
        assert!(end <= 0x1_0000);
    }

    #[test]
    fn opd_bytes_point_at_code_addr() {
        let code = u32::from_be_bytes([
            TRAMPOLINE_OPD_BYTES[0],
            TRAMPOLINE_OPD_BYTES[1],
            TRAMPOLINE_OPD_BYTES[2],
            TRAMPOLINE_OPD_BYTES[3],
        ]);
        let toc = u32::from_be_bytes([
            TRAMPOLINE_OPD_BYTES[4],
            TRAMPOLINE_OPD_BYTES[5],
            TRAMPOLINE_OPD_BYTES[6],
            TRAMPOLINE_OPD_BYTES[7],
        ]);
        assert_eq!(code, CALLBACK_RETURN_CODE_ADDR);
        assert_eq!(toc, 0);
    }

    #[test]
    fn trampoline_decodes_to_li_ori_sc() {
        let lis = u32::from_be_bytes([
            TRAMPOLINE_CODE_BYTES[0],
            TRAMPOLINE_CODE_BYTES[1],
            TRAMPOLINE_CODE_BYTES[2],
            TRAMPOLINE_CODE_BYTES[3],
        ]);
        let ori = u32::from_be_bytes([
            TRAMPOLINE_CODE_BYTES[4],
            TRAMPOLINE_CODE_BYTES[5],
            TRAMPOLINE_CODE_BYTES[6],
            TRAMPOLINE_CODE_BYTES[7],
        ]);
        let sc = u32::from_be_bytes([
            TRAMPOLINE_CODE_BYTES[8],
            TRAMPOLINE_CODE_BYTES[9],
            TRAMPOLINE_CODE_BYTES[10],
            TRAMPOLINE_CODE_BYTES[11],
        ]);
        // PPC `lis rD, SIMM`: opcode 15, RA=0; rD=11, SIMM=8.
        // Encoding: 001111 01011 00000 0000000000001000.
        assert_eq!(lis, (15 << 26) | (11 << 21) | 8);
        // PPC `ori rA, rS, UIMM`: opcode 24; rS=11, rA=11, UIMM=0.
        assert_eq!(ori, (24 << 26) | (11 << 21) | (11 << 16));
        // PPC `sc 0`: 010001 00000 00000 0000000000000010.
        assert_eq!(sc, (17 << 26) | 2);
    }

    /// The combined `lis; ori` sequence must materialize exactly
    /// `CB_RETURN_SYSCALL` in r11 -- this is the invariant
    /// Decision 1 (8/12-byte trampoline) defends.
    #[test]
    fn lis_ori_materializes_cb_return_syscall() {
        // lis r11, SIMM:  r11 = SIMM << 16
        let lis_simm: u64 = 8;
        let ori_uimm: u64 = 0;
        let after_lis = lis_simm << 16;
        // ori r11, r11, UIMM:  r11 = r11 | UIMM
        let after_ori = after_lis | ori_uimm;
        assert_eq!(after_ori, CB_RETURN_SYSCALL);
    }
}
