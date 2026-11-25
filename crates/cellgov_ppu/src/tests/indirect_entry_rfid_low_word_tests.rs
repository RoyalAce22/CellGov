use super::tests::{at, bound, word};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_NOP;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_IR, MSR_PR};

/// `mtspr spr,rs`.
///
/// [PPC-Book3 p:19 s:3.4] `mtspr` is primary opcode 31 with extended opcode 467, RS in bits 6:10 and the SPR number in bits 11:20 with its halves swapped.
fn mtspr(spr: u16, rs: u8) -> u32 {
    let spr = u32::from(spr);
    (31 << 26) | (u32::from(rs) << 21) | ((spr & 0x1F) << 16) | ((spr >> 5) << 11) | (467 << 1)
}

/// [PPC-Book3 p:119 s:Appendix G] `rfid` is an XL-form word with primary opcode 19 and extended opcode 18.
const RFID: u32 = (19 << 26) | (18 << 1);

/// `li rt,value`.
fn li(rt: u8, value: i16) -> u32 {
    word(PpuInstruction::Addi {
        rt,
        ra: 0,
        imm: value,
    })
}

/// `li r3,1; rldicr r3,r3,32,31; oris r3,r3,low >> 16; ori r3,r3,low`:
/// 0x1_0000_0000 | `low` in r3.
fn build_high(low: u64) -> [u32; 4] {
    [
        li(3, 1),
        word(PpuInstruction::Rldicr {
            ra: 3,
            rs: 3,
            sh: 32,
            me: 31,
            rc: false,
        }),
        word(PpuInstruction::Oris {
            ra: 3,
            rs: 3,
            imm: u16::try_from(low >> 16).expect("a low address"),
        }),
        word(PpuInstruction::Ori {
            ra: 3,
            rs: 3,
            imm: u16::try_from(low & 0xFFFF).expect("a halfword"),
        }),
    ]
}

/// The bound of code that builds 0x1_0000_0000 | `at(11)` in SRR0 and
/// the value that the three words `srr1` build in r4 in SRR1, then runs
/// `rfid` at word 9. The low-order 32 bits of SRR0 name word 11.
fn rfid_bound(srr1: [u32; 3]) -> BTreeSet<u64> {
    let [one, sldi, high, low] = build_high(at(11));
    let words = [
        one,
        sldi,
        high,
        low,
        srr1[0],
        srr1[1],
        srr1[2],
        mtspr(SPR_SRR0, 3),
        mtspr(SPR_SRR1, 4),
        RFID,
        PPC_NOP,
        PPC_NOP,
        PPC_NOP,
    ];
    bound(&words, false)
}

/// The words of a value in r4 that SRR1 can take.
fn r4(value: u64) -> [u32; 3] {
    [
        li(4, i16::try_from(value).expect("a low MSR bit")),
        PPC_NOP,
        PPC_NOP,
    ]
}

/// `li r4,1; rldicr r4,r4,sh,63-sh; ori r4,r4,MSR_IR`: MSR bit `63 - sh`
/// and IR in r4.
fn high_bit_and_ir(sh: u8) -> [u32; 3] {
    [
        li(4, 1),
        word(PpuInstruction::Rldicr {
            ra: 4,
            rs: 4,
            sh,
            me: 63 - sh,
            rc: false,
        }),
        word(PpuInstruction::Ori {
            ra: 4,
            rs: 4,
            imm: u16::try_from(MSR_IR).expect("a low MSR bit"),
        }),
    ]
}

/// An SRR1 that sets IR or PR with SF at 0 makes the `rfid` fetch from
/// the low-order 32 bits of SRR0. A loaded SRR1 can do so too.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` fetches from SRR0 0:61 || 0b00 when SF = 1 in the new MSR, and from 32 zeros || SRR0 32:61 || 0b00 when SF = 0.
/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\].
#[test]
fn an_rfid_that_can_keep_ir_with_sf_at_0_enters_the_low_word_of_srr0() {
    let ld = word(PpuInstruction::Ld {
        rt: 4,
        ra: 1,
        imm: 0,
    });
    for (srr1, text) in [
        (r4(MSR_IR), "IR"),
        (r4(MSR_PR), "PR"),
        ([ld, PPC_NOP, PPC_NOP], "loaded"),
    ] {
        assert_eq!(rfid_bound(srr1), BTreeSet::from([at(11)]), "{text}");
    }
}

/// With SF at 1 the `rfid` fetches from all of SRR0, which no segment
/// holds. SRR1 bit 0 or SRR1 bit 1 alone sets SF. With IR and PR at 0
/// the `rfid` puts control into real mode, which the real-mode switches
/// keep.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` fetches from SRR0 0:61 || 0b00 when SF = 1 in the new MSR, and from 32 zeros || SRR0 32:61 || 0b00 when SF = 0.
/// [PPC-Book3 p:13 s:2.3] `rfid` places the OR of SRR1 bits 0 and 1 into MSR\[SF\].
#[test]
fn an_rfid_with_srr1_bit_0_or_1_at_1_or_ir_at_0_enters_no_low_word() {
    let sf_ir = [
        word(PpuInstruction::Addis {
            rt: 4,
            ra: 0,
            imm: i16::MIN,
        }),
        word(PpuInstruction::Ori {
            ra: 4,
            rs: 4,
            imm: u16::try_from(MSR_IR).expect("a low MSR bit"),
        }),
        PPC_NOP,
    ];
    for (srr1, text) in [
        (sf_ir, "SF and IR"),
        (high_bit_and_ir(63), "bit 0 and IR"),
        (high_bit_and_ir(62), "bit 1 and IR"),
        (r4(0), "IR at 0"),
    ] {
        assert_eq!(rfid_bound(srr1), BTreeSet::new(), "{text}");
    }
}

/// Only the `b` brings SRR0's value to the `rfid`, because the `b` ends
/// the straight-line path. So only the scan that starts each branch
/// target with what its paths bring knows SRR0 at the `rfid`.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` fetches from SRR0 0:61 || 0b00 when SF = 1 in the new MSR, and from 32 zeros || SRR0 32:61 || 0b00 when SF = 0.
#[test]
fn an_rfid_past_a_branch_target_enters_the_low_word_that_the_branch_brings() {
    let [one, sldi, high, low] = build_high(at(12));
    let b = word(PpuInstruction::B {
        offset: 8,
        aa: false,
        link: false,
    });
    let [srr1, _, _] = r4(MSR_IR);
    let words = [
        one,
        sldi,
        high,
        low,
        b,
        PPC_NOP,
        srr1,
        mtspr(SPR_SRR0, 3),
        mtspr(SPR_SRR1, 4),
        RFID,
        PPC_NOP,
        PPC_NOP,
        PPC_NOP,
    ];
    assert_eq!(bound(&words, false), BTreeSet::from([at(12)]));
}

/// With SF at 0 the `rfid` fetches word 11, the low-order 32 bits of the
/// SRR0 that the `mtspr` writes before the `b`. The join pass carries no
/// SRR0 to the branch target, so the census enters no word for this
/// `rfid`.
///
/// [PPC-Book1 p:24 s:2.4] A branch alters only CTR and LR.
/// [PPC-Book3 p:13 s:2.3] `rfid` fetches from SRR0 0:61 || 0b00 when SF = 1 in the new MSR, and from 32 zeros || SRR0 32:61 || 0b00 when SF = 0.
#[test]
fn an_rfid_past_a_branch_misses_the_low_word_of_the_srr0_written_before_the_branch() {
    let [one, sldi, high, low] = build_high(at(11));
    let b = word(PpuInstruction::B {
        offset: 8,
        aa: false,
        link: false,
    });
    let [srr1, _, _] = r4(MSR_IR);
    let words = [
        one,
        sldi,
        high,
        low,
        mtspr(SPR_SRR0, 3),
        b,
        PPC_NOP,
        srr1,
        mtspr(SPR_SRR1, 4),
        RFID,
        PPC_NOP,
        PPC_NOP,
        PPC_NOP,
    ];
    assert_eq!(
        bound(&words, false),
        BTreeSet::new(),
        "pins a known gap: the join pass carries no SRR0, so the bound misses word 11"
    );
}
