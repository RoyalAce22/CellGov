use super::reach::Reach;
use super::tests::{at, pointer, word};
use super::*;
use cellgov_ps3_abi::format::elf::ELF64_E_ENTRY;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BLR, PPC_NOP};

/// The address of word 0 of the code.
const BASE: u64 = 0x1_0000;

/// A `bl` to a routine that the code does not hold.
fn call() -> u32 {
    word(PpuInstruction::B {
        offset: 0x1000,
        aa: false,
        link: true,
    })
}

fn bctr() -> u32 {
    word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    })
}

/// The `bctr` at word 1 follows a call that [`pointer`] reaches, so it
/// loses its target and can enter any word. Then the `bctr` at word 4,
/// after the `blr`, also runs, and it loses its target as well.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
#[test]
fn a_lost_target_lets_a_bctr_that_no_word_leads_to_lose_its_target_too() {
    let words = [call(), bctr(), PPC_BLR, call(), bctr()];
    let code = Code::new(vec![CodeSegment {
        vaddr: BASE,
        words: words.to_vec(),
    }]);
    let bound = indirect_entries(&[], &[pointer()], &code).expect("the code has no jump table");
    assert_eq!(bound.lost_targets, BTreeSet::from([at(1), at(4)]));
}

/// A start at 0x1_0000_7000 reaches the kernel word at 0x8000000000007000:
/// a `bcctr` in real mode with MSR\[SF\] at 0 takes the low-order 32
/// bits, 0x7000, and real mode ignores EA bits 0 and 1.
///
/// [PPC-Book1 p:20 s:2.4.1] In 32-bit mode a branch sets the high-order 32 bits of its target address to 0.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_start_reaches_the_word_at_the_real_address_of_its_low_word() {
    let code = Code::new(vec![CodeSegment {
        vaddr: 0x8000_0000_0000_7000,
        words: vec![PPC_NOP],
    }]);
    let mut reach = Reach::new(&code);
    assert!(reach.grow([0x1_0000_7000]));
    assert!(reach.contains(0, 0));
}

/// Whether control reaches each word of `words` at [`BASE`] from word 0.
fn reached(words: &[u32]) -> Vec<bool> {
    let code = Code::new(vec![CodeSegment {
        vaddr: BASE,
        words: words.to_vec(),
    }]);
    let mut reach = Reach::new(&code);
    reach.grow([BASE]);
    (0..words.len())
        .map(|index| reach.contains(0, index))
        .collect()
}

/// [PPC-Book1 p:24 s:2.4] `b` and `bc` reach CIA + EXTS(displacement), or EXTS(displacement) when AA = 1.
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
#[test]
fn a_b_reaches_only_its_target_and_a_bl_also_reaches_the_next_word() {
    let b = |link| {
        word(PpuInstruction::B {
            offset: 8,
            aa: false,
            link,
        })
    };
    assert_eq!(reached(&[b(false), PPC_NOP, PPC_NOP]), [true, false, true]);
    assert_eq!(reached(&[b(true), PPC_NOP, PPC_NOP]), [true, true, true]);
}

/// BO 0b10000 decrements CTR, and BO 0b00100 and 0b01100 test a CR bit,
/// so each of these can fall through.
///
/// [PPC-Book1 p:20 s:2.4.1] BO=1z1zz is unconditional.
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
#[test]
fn a_bc_that_branches_always_reaches_the_next_word_only_when_it_links() {
    let bc = |bo, link| {
        word(PpuInstruction::Bc {
            bo,
            bi: 2,
            offset: 8,
            aa: false,
            link,
        })
    };
    for bo in [0b10100, 0b11111] {
        let words = [bc(bo, false), PPC_NOP, PPC_NOP];
        assert_eq!(reached(&words), [true, false, true], "{bo:#07b}");
        let words = [bc(bo, true), PPC_NOP, PPC_NOP];
        assert_eq!(reached(&words), [true, true, true], "{bo:#07b} linked");
    }
    for bo in [0b10000, 0b00100, 0b01100] {
        let words = [bc(bo, false), PPC_NOP, PPC_NOP];
        assert_eq!(reached(&words), [true, true, true], "{bo:#07b}");
    }
}

/// [PPC-Book1 p:20 s:2.4.1] BO=1z1zz is unconditional.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
/// [PPC-Book1 p:25 s:2.4] `bclrl` and `bcctrl` set LR to CIA + 4.
#[test]
fn a_bclr_or_bcctr_reaches_the_next_word_only_when_it_links_or_tests_a_condition() {
    let bclr = |bo, link| word(PpuInstruction::Bclr { bo, bi: 2, link });
    let bcctr = |link| {
        word(PpuInstruction::Bcctr {
            bo: 0b10100,
            bi: 0,
            link,
        })
    };
    for (first, next, text) in [
        (bclr(0b10100, false), false, "blr"),
        (bcctr(false), false, "bctr"),
        (bclr(0b10100, true), true, "blrl"),
        (bcctr(true), true, "bctrl"),
        (bclr(0b01100, false), true, "beqlr"),
    ] {
        assert_eq!(reached(&[first, PPC_NOP]), [true, next], "{text}");
    }
}

/// The word after 0xFFFF_FFFC is at 0x1_0000_0000, which no segment
/// holds. Its low-order 32 bits are 0, where the next segment starts.
///
/// [PPC-Book1 p:15 s:1.12.2] In 32-bit mode the effective address of an instruction fetch wraps from 2^32 - 1 to 0.
#[test]
fn a_fall_through_past_the_last_word_below_4_gib_reaches_address_0() {
    let code = Code::new(vec![
        CodeSegment {
            vaddr: 0xFFFF_FFFC,
            words: vec![PPC_NOP],
        },
        CodeSegment {
            vaddr: 0,
            words: vec![PPC_NOP],
        },
    ]);
    let mut reach = Reach::new(&code);
    assert!(reach.grow([0xFFFF_FFFC]));
    assert!(reach.contains(1, 0));
}

/// The low-order 32 bits of the start name no word. So only the start
/// with its bits 62 and 63 at 0 reaches the word.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_start_with_its_low_two_bits_set_reaches_the_word_that_holds_it() {
    let code = Code::new(vec![CodeSegment {
        vaddr: 0x1_0000_0000,
        words: vec![PPC_NOP],
    }]);
    let mut reach = Reach::new(&code);
    assert!(reach.grow([0x1_0000_0003]));
    assert!(reach.contains(0, 0));
}

/// The two segments differ only in EA bit 0, so they have one real
/// address. The low-order 32 bits, 0x7000, name no word.
///
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_start_reaches_each_segment_at_its_real_address() {
    let code = Code::new(vec![
        CodeSegment {
            vaddr: 0x1_0000_7000,
            words: vec![PPC_NOP],
        },
        CodeSegment {
            vaddr: 0x8000_0001_0000_7000,
            words: vec![PPC_NOP],
        },
    ]);
    let mut reach = Reach::new(&code);
    assert!(reach.grow([0x1_0000_7000]));
    assert!(reach.contains(0, 0));
    assert!(reach.contains(1, 0));
}

#[test]
fn grow_and_fill_return_whether_the_set_took_a_word() {
    let code = Code::new(vec![CodeSegment {
        vaddr: BASE,
        words: vec![PPC_NOP, PPC_BLR, PPC_NOP],
    }]);
    let mut reach = Reach::new(&code);
    assert!(reach.grow([BASE]));
    assert!(!reach.grow([BASE + 4]));
    assert!(!reach.grow([BASE + 0x1000]));
    assert!(!reach.contains(0, 2));
    assert!(reach.fill());
    assert!(reach.contains(0, 2));
    assert!(!reach.fill());
    assert!(!reach.grow([BASE + 8]));
    let all = Code::new(vec![CodeSegment {
        vaddr: BASE,
        words: vec![PPC_NOP, PPC_BLR],
    }]);
    let mut every = Reach::new(&all);
    assert!(every.grow([BASE]));
    assert!(!every.fill());
}

/// The ELF entry is word 1, and control does not reach word 0. The path
/// from word 1 brings 0x28 in r5 over the `b` to word 4, where `oris`
/// builds the address of word 10.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` and `addis` add EXTS(SI) or EXTS(SI || 0x0000) to RA, or to 0 when RA is 0.
/// [PPC-Book1 p:66 s:3.3.11] `oris` ORs RS with UI shifted left 16 bits and places the result into RA.
/// [PPC-Book1 p:24 s:2.4] A branch alters only CTR and LR.
#[test]
fn a_path_from_a_reached_word_after_an_unreached_word_brings_its_constants_over_a_branch() {
    let mut words = vec![
        PPC_NOP,
        word(PpuInstruction::Addi {
            rt: 5,
            ra: 0,
            imm: 0x28,
        }),
        word(PpuInstruction::B {
            offset: 8,
            aa: false,
            link: false,
        }),
        PPC_NOP,
        word(PpuInstruction::Oris {
            ra: 5,
            rs: 5,
            imm: 1,
        }),
        PPC_BLR,
    ];
    words.resize(12, PPC_NOP);
    let code = Code::new(vec![CodeSegment { vaddr: BASE, words }]);
    let mut elf = [0u8; ELF64_E_ENTRY + 8];
    elf[ELF64_E_ENTRY..].copy_from_slice(&at(1).to_be_bytes());
    let bound = indirect_entries(&elf, &[], &code).expect("the code has no jump table");
    assert_eq!(bound.entries, BTreeSet::from([at(1), at(10)]));
}
