//! `twi` and `tdi`: decode, the five TO conditions against the
//! sign-extended immediate, and agreement with `tw` and `td`.
//! [PPC-Book1 p:63 s:3.3.10] TO bits 0-4 select signed <, signed >, =, unsigned < and unsigned >.

use super::*;

fn twi(to: u8, imm: i16) -> PpuInstruction {
    PpuInstruction::Twi { to, ra: 3, imm }
}

fn tdi(to: u8, imm: i16) -> PpuInstruction {
    PpuInstruction::Tdi { to, ra: 3, imm }
}

/// Whether `insn` traps with RA (r3) = `a` and r4 = `b`.
fn traps(insn: &PpuInstruction, a: u64, b: u64) -> bool {
    let mut s = PpuState::new();
    s.set_gpr(3, a);
    s.set_gpr(4, b);
    match exec_no_mem(insn, &mut s) {
        ExecuteVerdict::Continue => false,
        ExecuteVerdict::Fault(PpuFault::ProgramTrap(to)) => {
            let want = match *insn {
                PpuInstruction::Twi { to, .. }
                | PpuInstruction::Tdi { to, .. }
                | PpuInstruction::Tw { to, .. }
                | PpuInstruction::Td { to, .. } => to,
                _ => unreachable!(),
            };
            assert_eq!(to, want);
            true
        }
        other => panic!("{insn:?}: {other:?}"),
    }
}

#[test]
fn both_primaries_decode_and_round_trip() {
    for (primary, to, ra, imm) in [(3u32, 4u8, 5u8, -2i16), (2, 24, 31, 0x7ff0), (3, 31, 0, 1)] {
        let raw =
            (primary << 26) | (u32::from(to) << 21) | (u32::from(ra) << 16) | u32::from(imm as u16);
        let insn = crate::decode::decode(raw).unwrap_or_else(|e| panic!("{raw:#010x}: {e:?}"));
        let want = if primary == 3 {
            PpuInstruction::Twi { to, ra, imm }
        } else {
            PpuInstruction::Tdi { to, ra, imm }
        };
        assert_eq!(insn, want);
        assert_eq!(crate::instruction::encode::encode(&insn), Ok(raw));
    }
}

#[test]
fn each_to_bit_alone_traps_only_when_its_condition_holds() {
    // (TO, RA that holds, RA that does not), against SI = 5.
    for (to, holds, fails) in [
        (16, (-3i64) as u64, 5),
        (8, 7, 5),
        (4, 5, 6),
        (2, 4, (-1i64) as u64),
        (1, (-1i64) as u64, 4),
    ] {
        for make in [twi, tdi] {
            assert!(
                traps(&make(to, 5), holds, 0),
                "{:?} RA {holds:#x}",
                make(to, 5)
            );
            assert!(
                !traps(&make(to, 5), fails, 0),
                "{:?} RA {fails:#x}",
                make(to, 5)
            );
        }
    }
}

#[test]
fn unsigned_conditions_compare_against_the_sign_extended_immediate() {
    // SI = -1 extends to the largest unsigned value.
    for a in [0, 1, 0x7FFF_FFFF_FFFF_FFFF, 0xFFFF_FFFF_FFFF_FFFE] {
        assert!(traps(&tdi(2, -1), a, 0), "{a:#x}");
    }
    assert!(!traps(&tdi(2, -1), u64::MAX, 0));
}

#[test]
fn twi_compares_the_low_word_and_tdi_all_64_bits() {
    // 0x1_0000_0005: low word 5, whole register far above 5.
    let a = 0x0000_0001_0000_0005;
    assert!(traps(&twi(4, 5), a, 0));
    assert!(!traps(&tdi(4, 5), a, 0));
    assert!(traps(&tdi(8, 5), a, 0));
    // RA values that differ only in the high word give twi the same answer.
    for to in 0..32 {
        assert_eq!(
            traps(&twi(to, -7), 0xAAAA_AAAA_FFFF_FFF9, 0),
            traps(&twi(to, -7), 0x5555_5555_FFFF_FFF9, 0),
            "TO {to}"
        );
    }
}

#[test]
fn to_0_never_traps_and_to_31_always_traps() {
    for a in [0, 1, u64::MAX, 0x8000_0000, 0x8000_0000_0000_0000] {
        for imm in [i16::MIN, -1, 0, 1, i16::MAX] {
            assert!(!traps(&twi(0, imm), a, 0));
            assert!(!traps(&tdi(0, imm), a, 0));
            assert!(traps(&twi(31, imm), a, 0));
            assert!(traps(&tdi(31, imm), a, 0));
        }
    }
}

#[test]
fn the_immediate_forms_agree_with_the_register_forms() {
    const EDGES: [u64; 7] = [
        0,
        1,
        u64::MAX,
        0x7FFF_FFFF,
        0x8000_0000,
        0x7FFF_FFFF_FFFF_FFFF,
        0x8000_0000_0000_0000,
    ];
    let mut compared = 0;
    for to in 0..32 {
        for a in EDGES {
            for imm in [i16::MIN, -1, 0, 1, i16::MAX] {
                let b = i64::from(imm) as u64;
                let tw = PpuInstruction::Tw { to, ra: 3, rb: 4 };
                let td = PpuInstruction::Td { to, ra: 3, rb: 4 };
                assert_eq!(
                    traps(&twi(to, imm), a, 0),
                    traps(&tw, a, b),
                    "twi {to} {a:#x} {imm}"
                );
                assert_eq!(
                    traps(&tdi(to, imm), a, 0),
                    traps(&td, a, b),
                    "tdi {to} {a:#x} {imm}"
                );
                compared += 1;
            }
        }
    }
    assert_eq!(compared, 32 * 7 * 5);
}

#[test]
fn the_immediate_forms_disassemble() {
    for (insn, want) in [
        (twi(4, -2), "twi 4, r3, -2"),
        (tdi(24, 16), "tdi 24, r3, 16"),
    ] {
        let text = crate::instruction::AsmText {
            insn: &insn,
            addr: 0,
            symbols: None,
        }
        .to_string();
        assert_eq!(text.split_whitespace().collect::<Vec<_>>().join(" "), want);
    }
}
