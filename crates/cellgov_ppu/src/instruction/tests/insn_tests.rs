//! PpuInstruction static-name derivation matches Debug variant identifiers.

use super::*;

#[test]
fn variant_name_matches_debug_prefix() {
    let cases: &[PpuInstruction] = &[
        PpuInstruction::Addi {
            rt: 0,
            ra: 0,
            imm: 0,
        },
        PpuInstruction::Lwz {
            rt: 0,
            ra: 0,
            imm: 0,
        },
        PpuInstruction::B {
            offset: 0,
            aa: false,
            link: false,
        },
        PpuInstruction::Sc { lev: 0 },
        PpuInstruction::Fp63 {
            xo: 0,
            frt: 0,
            fra: 0,
            frb: 0,
            frc: 0,
            rc: false,
        },
        PpuInstruction::Consumed,
        PpuInstruction::Lwa {
            rt: 0,
            ra: 0,
            imm: 0,
        },
        PpuInstruction::AddicDot {
            rt: 0,
            ra: 0,
            imm: 0,
        },
        PpuInstruction::AndisDot {
            ra: 0,
            rs: 0,
            imm: 0,
        },
    ];
    for insn in cases {
        let debug = format!("{insn:?}");
        let prefix = debug
            .split_once([' ', '{'])
            .map(|(n, _)| n)
            .unwrap_or(&debug);
        let name: &'static str = insn.into();
        assert_eq!(
            name, prefix,
            "IntoStaticStr-derived name mismatch for {debug}",
        );
    }
}

#[test]
fn into_static_str_returns_variant_ident() {
    let insn = PpuInstruction::Add {
        rt: 3,
        ra: 4,
        rb: 5,
        oe: false,
        rc: false,
    };
    let name: &'static str = (&insn).into();
    assert_eq!(name, "Add");
}
