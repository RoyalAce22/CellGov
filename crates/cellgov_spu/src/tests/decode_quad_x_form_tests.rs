//! The x-form quadword load and store decode on their whole 11-bit
//! opcode, so their unassigned neighbours and `stopd` are not loads or
//! stores.

use super::*;

/// Every 11-bit opcode under one 8-bit prefix, with fixed register
/// fields, decoded.
fn under_prefix(prefix: u32) -> Vec<(u32, Result<SpuInstruction, SpuDecodeError>)> {
    (0..8)
        .map(|low| {
            let op11 = (prefix << 3) | low;
            let raw = (op11 << 21) | (3 << 14) | (2 << 7) | 1;
            (op11, decode(raw))
        })
        .collect()
}

// [SPU-ISA p:33 s:3] lqx is 00111000100.
// [SPU-ISA p:259 s:A] Table A-1 starts the list of every SPU instruction; checked entry by entry, only lqx starts 00111000.
#[test]
fn only_lqx_decodes_under_its_prefix() {
    for (op11, decoded) in under_prefix(0x38) {
        if op11 == 0x1C4 {
            assert_eq!(
                decoded,
                Ok(SpuInstruction::Lqx {
                    rt: 1,
                    ra: 2,
                    rb: 3
                })
            );
        } else {
            assert!(
                matches!(decoded, Err(SpuDecodeError::Unassigned(_))),
                "op11 0x{op11:03x} decoded as {decoded:?}",
            );
        }
    }
}

// [SPU-ISA p:37 s:3] stqx is 00101000100; [SPU-ISA p:239 s:10] stopd is 00101000000.
// [SPU-ISA p:259 s:A] Table A-1 starts the list of every SPU instruction; checked entry by entry, only stqx and stopd start 00101000.
#[test]
fn only_stqx_and_stopd_decode_under_their_prefix() {
    for (op11, decoded) in under_prefix(0x28) {
        match op11 {
            0x144 => assert_eq!(
                decoded,
                Ok(SpuInstruction::Stqx {
                    rt: 1,
                    ra: 2,
                    rb: 3
                })
            ),
            0x140 => assert_eq!(decoded, Ok(SpuInstruction::Stopd)),
            _ => assert!(
                matches!(decoded, Err(SpuDecodeError::Unassigned(_))),
                "op11 0x{op11:03x} decoded as {decoded:?}",
            ),
        }
    }
}
