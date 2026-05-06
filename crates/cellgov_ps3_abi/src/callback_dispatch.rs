//! Worker-thread callback-dispatch primitive ABI constants.
//!
//! CellGov synthesizes a guest-visible re-entry trampoline so HLE
//! handlers can invoke a title-supplied function pointer on a worker
//! PPU thread and resume only after the worker returns. The worker's
//! terminal `blr` lands on the trampoline, which issues a
//! CellGov-private LV2 syscall classified as a callback return.
//!
//! # Trampoline layout
//!
//! 32 bytes at [`CALLBACK_RETURN_REGION_BASE`], inside the
//! pre-user-heap scratch zone (`0..0x10000`) of the main user-memory
//! region. PPU instruction fetch only reads the base-0 region, so the
//! trampoline must live there to be executable.
//!
//! | Offset | Size | Contents                                            |
//! | ------ | ---- | --------------------------------------------------- |
//! | 0      | 4    | `lis r11, 8`        (high half of `CB_RETURN_SYSCALL`) |
//! | 4      | 4    | `ori r11, r11, 0`   (low half; combined: `r11 = 0x80000`) |
//! | 8      | 4    | `sc 0`              (CellGov-private syscall)       |
//! | 12     | 8    | OPD `(code_addr = 0x0000_FF00, toc = 0)`            |
//! | 20     | 12   | reserved (zero-filled padding)                      |
//!
//! HLE handlers stage [`CALLBACK_RETURN_CODE_ADDR`] into a worker
//! thread's LR; the worker's terminal `blr` sets `PC = LR` and lands
//! on the trampoline body. The [`CALLBACK_RETURN_OPD_ADDR`] OPD slot
//! is for consumers that need a callable function-pointer handle
//! (e.g. flip-handler / vblank-handler). Reloading `r11` inside the
//! trampoline removes any dependency on `r11` surviving `blr` from
//! the title's callback (PPC64 ELFv1 marks `r11` volatile).

/// CellGov-private syscall number issued by the callback-return
/// trampoline. Routed to `Lv2Request::CallbackDispatchReturn`.
pub const CB_RETURN_SYSCALL: u64 =
    crate::syscall_namespace::CellGovPrivateSyscall::CallbackReturn.encode();

/// Lowest address of the 32-byte callback-return region.
pub const CALLBACK_RETURN_REGION_BASE: u32 = 0x0000_FF00;

/// Reserved region size in bytes. Code (12) + OPD (8) + pad (12).
pub const CALLBACK_RETURN_REGION_SIZE: u32 = 32;

/// Address of the trampoline code (`lis; ori; sc 0`).
pub const CALLBACK_RETURN_CODE_ADDR: u32 = 0x0000_FF00;

/// Address of the OPD slot.
pub const CALLBACK_RETURN_OPD_ADDR: u32 = 0x0000_FF0C;

/// Maximum recursion depth for nested callbacks.
pub const CALLBACK_DEPTH_CAP: u8 = 8;

/// Big-endian PPC64 instruction bytes for the trampoline body
/// (12 bytes: `lis r11, 8; ori r11, r11, 0; sc 0`).
///
/// `CB_RETURN_SYSCALL` is below `0x100000` (its namespace upper
/// bound), so the `as u32` narrowing cannot lose data.
pub const TRAMPOLINE_CODE_BYTES: [u8; 12] =
    crate::trampoline_codegen::encode_lis_ori_sc(CB_RETURN_SYSCALL as u32);

/// Big-endian RPCS3-packed OPD bytes: code_addr =
/// `CALLBACK_RETURN_CODE_ADDR`, toc = 0. Matches CellGov's HLE
/// thunk layout (8-byte `(u32 code, u32 toc)` pair); not the 24-
/// byte PPC64 ELFv1 OPD shape.
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
        assert_eq!(lis, (15 << 26) | (11 << 21) | 8);
        assert_eq!(ori, (24 << 26) | (11 << 21) | (11 << 16));
        assert_eq!(sc, (17 << 26) | 2);
    }

    #[test]
    fn lis_ori_materializes_cb_return_syscall() {
        let lis_simm: u64 = 8;
        let ori_uimm: u64 = 0;
        let after_lis = lis_simm << 16;
        let after_ori = after_lis | ori_uimm;
        assert_eq!(after_ori, CB_RETURN_SYSCALL);
    }
}
