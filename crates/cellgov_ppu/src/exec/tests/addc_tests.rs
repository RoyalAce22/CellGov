//! `addc` and its OE and Rc forms.
//! [PPC-Book1 p:53 s:3.3.8] RT <- (RA) + (RB); CA is the carry out, CR0 is set when Rc=1, SO and OV when OE=1.

use super::*;

const XER_SO: u64 = 1 << 31;
const XER_OV: u64 = 1 << 30;

fn addc(oe: bool, rc: bool) -> PpuInstruction {
    PpuInstruction::Addc {
        rt: 5,
        ra: 3,
        rb: 4,
        oe,
        rc,
    }
}

fn run(insn: &PpuInstruction, a: u64, b: u64, xer: u64) -> PpuState {
    let mut s = PpuState::new();
    s.set_gpr(3, a);
    s.set_gpr(4, b);
    s.set_xer(xer);
    exec_no_mem(insn, &mut s);
    s
}

#[test]
fn the_four_forms_decode_to_addc_and_round_trip() {
    for (oe, rc, text) in [
        (false, false, "addc"),
        (false, true, "addc."),
        (true, false, "addco"),
        (true, true, "addco."),
    ] {
        let raw = (31 << 26)
            | (5 << 21)
            | (3 << 16)
            | (4 << 11)
            | (u32::from(oe) << 10)
            | (10 << 1)
            | u32::from(rc);
        let insn = crate::decode::decode(raw).unwrap_or_else(|e| panic!("{text}: {e:?}"));
        assert_eq!(insn, addc(oe, rc), "{text}");
        assert_eq!(crate::instruction::encode::encode(&insn), Ok(raw), "{text}");
        let asm = crate::instruction::AsmText {
            insn: &insn,
            addr: 0,
            symbols: None,
        }
        .to_string();
        let asm = asm.split_whitespace().collect::<Vec<_>>().join(" ");
        assert_eq!(asm, format!("{text} r5, r3, r4"));
    }
}

#[test]
fn a_carry_out_sets_ca_and_none_clears_it() {
    let s = run(&addc(false, false), u64::MAX, 1, 0);
    assert_eq!(s.gpr[5], 0);
    assert!(s.xer_ca());
    let s = run(&addc(false, false), 5, 3, 1 << 29);
    assert_eq!(s.gpr[5], 8);
    assert!(!s.xer_ca());
}

#[test]
fn signed_overflow_sets_ov_and_so_only_with_oe() {
    let s = run(&addc(true, false), i64::MAX as u64, 1, 0);
    assert_eq!(s.gpr[5], 0x8000_0000_0000_0000);
    assert_eq!(s.xer() & (XER_OV | XER_SO), XER_OV | XER_SO);
    assert!(!s.xer_ca());
    let s = run(&addc(false, false), i64::MAX as u64, 1, 0);
    assert_eq!(s.xer() & (XER_OV | XER_SO), 0);
    // OE=1 with no overflow clears OV and leaves SO set.
    let s = run(&addc(true, false), 1, 1, XER_OV | XER_SO);
    assert_eq!(s.xer() & (XER_OV | XER_SO), XER_SO);
}

#[test]
fn rc_records_the_sign_of_the_result_and_copies_so() {
    for (a, b, field) in [
        (u64::MAX, u64::MAX, 0b1000),
        (2, 3, 0b0100),
        (u64::MAX, 1, 0b0010),
    ] {
        let s = run(&addc(false, true), a, b, 0);
        assert_eq!(s.cr_field(0), field, "{a:#x} + {b:#x}");
        let s = run(&addc(false, true), a, b, XER_SO);
        assert_eq!(s.cr_field(0), field | 1, "{a:#x} + {b:#x} with SO");
    }
}

#[test]
fn addc_and_adde_with_ca_clear_agree() {
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
    for a in EDGES {
        for b in EDGES {
            for (oe, rc) in [(false, false), (true, true)] {
                let adde = PpuInstruction::Adde {
                    rt: 5,
                    ra: 3,
                    rb: 4,
                    oe,
                    rc,
                };
                let x = run(&addc(oe, rc), a, b, XER_SO);
                let y = run(&adde, a, b, XER_SO);
                assert_eq!(
                    (x.gpr[5], x.xer(), x.cr()),
                    (y.gpr[5], y.xer(), y.cr()),
                    "{a:#x} + {b:#x}"
                );
                compared += 1;
            }
        }
    }
    assert_eq!(compared, 98);
}
