use super::hypercall_entry_tests::word;
use super::hypercall_real_mode_switch_tests::{msr_stub, mtmsrd, ori, rfid_stub, site, SC};
use super::hypercall_tests::li;
use super::*;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_FP, MSR_IR};

/// `mfmsr rt`.
///
/// [PPC-Book3 p:119 s:Appendix G] `mfmsr` is an X-form word with primary opcode 31 and extended opcode 83.
fn mfmsr(rt: u8) -> u32 {
    (31 << 26) | (u32::from(rt) << 21) | (83 << 1)
}

/// [PPC-Book3 p:23 s:3.4] `mfmsr` places the MSR into RT.
/// [PPC-Book1 p:66 s:3.3.11] `ori` ORs RS with 0x0000 || UI into RA.
/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\].
#[test]
fn an_rfid_whose_srr1_ors_ir_into_the_msr_puts_no_target_in_real_mode() {
    let stub = || rfid_stub(&[mfmsr(4), ori(4, MSR_IR)], &[]);
    assert_eq!(site(76, stub()), [(SC, Some(75))]);
}

/// The SRR1 holds the MSR of the word, which can have IR at 0, and the
/// `rfid` puts control at SRR0.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\] and fetches the next instruction from SRR0.
#[test]
fn an_rfid_whose_srr1_is_the_msr_puts_its_target_in_real_mode() {
    let stub = || rfid_stub(&[mfmsr(4)], &[]);
    assert_eq!(site(76, stub()), [(SC, None)]);
    assert_eq!(site(75, stub()), [(SC, Some(75))]);
}

/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\].
/// [PPC-Book3 p:65 s:5.5] Figure 35: each interrupt sets MSR\[IR\] to 0, and sets MSR\[PR\] to 0.
#[test]
fn an_mtmsrd_of_the_msr_with_fp_set_leaves_the_mode_as_it_was() {
    let stub = || msr_stub(&[mfmsr(5), ori(5, MSR_FP)], mtmsrd(5, false));
    assert_eq!(site(76, stub()), [(SC, Some(75))]);
}

/// [PPC-Book1 p:68 s:3.3.11] `andc` ANDs RS with the complement of RB into RA.
/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\].
#[test]
fn an_mtmsrd_of_the_msr_with_ir_cleared_puts_the_next_word_in_real_mode() {
    let clear_ir = word(PpuInstruction::Andc {
        ra: 5,
        rs: 5,
        rb: 6,
        rc: false,
    });
    let ir = i16::try_from(MSR_IR).expect("a low MSR bit");
    let stub = || msr_stub(&[li(6, ir), mfmsr(5), clear_ir], mtmsrd(5, false));
    assert_eq!(site(76, stub()), [(SC, None)]);
    assert_eq!(site(75, stub()), [(SC, Some(75))]);
}

/// The `sc` writes SRR1, so the `rfid` after it can clear IR.
///
/// [PPC-Book3 p:73 s:5.5.13] A System Call interrupt sets SRR0 and SRR1.
/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\].
#[test]
fn an_rfid_after_an_sc_takes_no_bit_of_the_srr1_before_it() {
    let sc = word(PpuInstruction::Sc { lev: 0 });
    let stub = || rfid_stub(&[mfmsr(4), ori(4, MSR_IR)], &[sc]);
    assert_eq!(site(76, stub()), [(SC, None)]);
    assert_eq!(site(75, stub()), [(SC, Some(75))]);
}

/// The routine of the call can write MSR, so the `mtmsrd` can clear IR.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
#[test]
fn an_mtmsrd_of_the_msr_read_before_a_call_puts_the_next_word_in_real_mode() {
    let call = word(PpuInstruction::B {
        offset: 4,
        aa: false,
        link: true,
    });
    let stub = || msr_stub(&[mfmsr(5), call], mtmsrd(5, false));
    assert_eq!(site(76, stub()), [(SC, None)]);
    assert_eq!(site(75, stub()), [(SC, Some(75))]);
}
