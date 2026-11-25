use super::*;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BL_OPCODE_LK, PPC_NOP};

/// `li r11, value`.
///
/// [PPC-Book1 p:162 s:B.9 Load Immediate] li Rx,value == addi Rx,0,value.
fn li_r11(value: u16) -> u32 {
    (14 << 26) | (11 << 21) | u32::from(value)
}

/// The ordinal of the one site in `words`.
fn only_ordinal(words: &[u32]) -> Option<u16> {
    let code = Code::new(vec![CodeSegment {
        vaddr: 0x1_0000,
        words: words.to_vec(),
    }]);
    let sites = scan(&code, &IndirectBound::default(), &BTreeSet::new());
    assert_eq!(sites.len(), 1, "{sites:?}");
    sites[0].ordinal
}

#[test]
fn a_walk_past_its_word_limit_leaves_the_site_unresolved() {
    let path = |nops: usize| {
        let mut words = vec![li_r11(74)];
        words.extend(std::iter::repeat_n(PPC_NOP, nops));
        words.push(PPC_SC_1);
        words
    };
    // The walk visits `sc` and each nop; the `li r11` ends the path.
    assert_eq!(only_ordinal(&path(HYPERCALL_WALK_LIMIT - 1)), Some(74));
    assert_eq!(only_ordinal(&path(HYPERCALL_WALK_LIMIT)), None);
}

#[test]
fn a_call_between_the_load_and_sc_leaves_the_site_unresolved() {
    let call = PPC_BL_OPCODE_LK | 0x100;
    assert_eq!(only_ordinal(&[li_r11(74), PPC_NOP, PPC_SC_1]), Some(74));
    assert_eq!(only_ordinal(&[li_r11(74), call, PPC_SC_1]), None);
}

fn word(instruction: PpuInstruction) -> u32 {
    crate::instruction::encode::encode(&instruction).expect("the instruction encodes")
}

#[test]
fn a_return_or_a_linking_conditional_branch_before_sc_leaves_the_site_unresolved() {
    let bc = |bo: u8, link: bool| {
        word(PpuInstruction::Bc {
            bo,
            bi: 0,
            offset: 0x100,
            aa: false,
            link,
        })
    };
    let blr = word(PpuInstruction::Bclr {
        bo: 0b10100,
        bi: 0,
        link: false,
    });
    let bctrl = word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: true,
    });
    assert_eq!(
        only_ordinal(&[li_r11(74), bc(0b00100, false), PPC_SC_1]),
        Some(74)
    );
    assert_eq!(
        only_ordinal(&[li_r11(74), bc(0b00100, true), PPC_SC_1]),
        None
    );
    assert_eq!(only_ordinal(&[li_r11(74), blr, PPC_SC_1]), None);
    assert_eq!(only_ordinal(&[li_r11(74), bctrl, PPC_SC_1]), None);
}

#[test]
fn a_word_that_may_write_r11_between_the_load_and_sc_leaves_the_site_unresolved() {
    let lwz = |rt: u8| word(PpuInstruction::Lwz { rt, ra: 1, imm: 0 });
    let lmw = word(PpuInstruction::Lmw {
        rt: 5,
        ra: 1,
        imm: 0,
    });
    assert_eq!(only_ordinal(&[li_r11(74), lwz(12), PPC_SC_1]), Some(74));
    assert_eq!(only_ordinal(&[li_r11(74), lwz(11), PPC_SC_1]), None);
    assert_eq!(only_ordinal(&[li_r11(74), lmw, PPC_SC_1]), None);
}
