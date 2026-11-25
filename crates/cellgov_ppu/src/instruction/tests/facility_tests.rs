use super::Facility;
use crate::decode::decode;
use crate::instruction::{AsmText, PpuInstruction};

/// The facility that the instruction with mnemonic `mnemonic` needs.
///
/// [PPC-Book1 p:86 s:4.2] The floating-point instructions do arithmetic, rounding, conversion and comparison, move data between storage and the FPRs, and move the FPSCR.
/// [CBE-Handbook p:740 s:A.3.2] Table A-8 lists the vector instructions, and `dst`, `dstst` and `dss` are no-ops.
///
/// Each floating-point mnemonic starts with `f`, `lf` or `stf`, or names
/// the FPSCR. Each vector mnemonic starts with `v`, `lv` or `stv`, or
/// moves the VSCR.
fn expected(mnemonic: &str) -> Option<Facility> {
    let m = mnemonic.trim_end_matches('.');
    let fpscr = matches!(
        m,
        "mffs" | "mcrfs" | "mtfsf" | "mtfsfi" | "mtfsb0" | "mtfsb1"
    );
    let vscr = matches!(m, "mfvscr" | "mtvscr");
    if m.starts_with('f') || m.starts_with("lf") || m.starts_with("stf") || fpscr {
        Some(Facility::FloatingPoint)
    } else if m.starts_with('v') || m.starts_with("lv") || m.starts_with("stv") || vscr {
        Some(Facility::Vector)
    } else {
        None
    }
}

/// Every word with operand fields 1, 2 and 3 and any primary opcode and
/// low 11 bits, which covers each form's extended opcode.
fn swept_words() -> impl Iterator<Item = u32> {
    (0..64u32).flat_map(|primary| {
        (0..2048u32).map(move |low| (primary << 26) | (1 << 21) | (2 << 16) | (3 << 11) | low)
    })
}

/// The decode of `raw` and its mnemonic, or `None` for a word that
/// decodes to no instruction the PPE implements.
///
/// [PPC-Book1 p:14 s:1.10] An optional instruction the implementation does not provide invokes the system illegal instruction error handler.
fn mnemonic(raw: u32) -> Option<(PpuInstruction, String)> {
    let insn = decode(raw).ok()?;
    if matches!(insn, PpuInstruction::PpeIllegal { .. }) {
        return None;
    }
    let text = AsmText {
        insn: &insn,
        addr: 0,
        symbols: None,
    }
    .to_string();
    let first = text.split_whitespace().next()?.to_string();
    Some((insn, first))
}

#[test]
fn each_decoded_word_needs_the_facility_its_mnemonic_names() {
    let (mut fp, mut vector) = (0u32, 0u32);
    for raw in swept_words() {
        let Some((insn, m)) = mnemonic(raw) else {
            continue;
        };
        let want = expected(&m);
        assert_eq!(insn.facility(), want, "{m} (0x{raw:08x})");
        match want {
            Some(Facility::FloatingPoint) => fp += 1,
            Some(Facility::Vector) => vector += 1,
            None => {}
        }
    }
    assert!(fp > 500, "{fp} floating-point words");
    assert!(vector > 500, "{vector} vector words");
}

/// [AltiVec-PEM p:2-9 s:2.5] The data stream instructions and the VRSAVE moves run with the vector unit disabled.
#[test]
fn the_data_stream_and_vrsave_moves_need_no_facility() {
    // dst r1,r2,0 / dstst r1,r2,0 / dss 0 / mfspr r1,256 / mtspr 256,r1.
    for raw in [
        0x7C01_12AC,
        0x7C01_12EC,
        0x7C00_066C,
        0x7C20_42A6,
        0x7C20_43A6,
    ] {
        let (insn, m) = mnemonic(raw).expect("a decoded word");
        assert_eq!(insn.facility(), None, "{m}");
    }
}

#[test]
fn each_decoded_word_is_a_branch_exactly_when_its_mnemonic_is_one() {
    let mut branches = 0u32;
    for raw in swept_words() {
        let Some((insn, m)) = mnemonic(raw) else {
            continue;
        };
        // [PPC-Book1 p:20 s:2.4.1] The branch instructions are Branch, Branch Conditional, and Branch Conditional to the Link Register or to the Count Register.
        // Their mnemonics and extended mnemonics each start with `b`.
        let want = m.starts_with('b');
        assert_eq!(insn.is_branch(), want, "{m} (0x{raw:08x})");
        branches += u32::from(want);
    }
    assert!(branches > 100, "{branches} branch words");
}

/// The decoder never yields a quickened form, so the sweep above does not
/// reach these.
///
/// [PPC-Book1 p:20 s:2.4.1] BO encodings: 001at branches if the CR bit is 0, 011at if it is 1, and 1a00t decrements CTR and branches if CTR != 0.
#[test]
fn each_quickened_branch_is_a_branch() {
    let quickened = [
        PpuInstruction::BcTrue {
            bo: 0b01100,
            bi: 2,
            offset: 8,
        },
        PpuInstruction::BcFalse {
            bo: 0b00100,
            bi: 2,
            offset: 8,
        },
        PpuInstruction::Bdnz {
            bo: 0b10000,
            bi: 0,
            offset: 8,
        },
        PpuInstruction::Blr { bo: 0b10100, bi: 0 },
    ];
    for insn in quickened {
        let text = AsmText {
            insn: &insn,
            addr: 0,
            symbols: None,
        }
        .to_string();
        assert!(text.starts_with('b'), "{text}");
        assert!(insn.is_branch(), "{text}");
    }
}
