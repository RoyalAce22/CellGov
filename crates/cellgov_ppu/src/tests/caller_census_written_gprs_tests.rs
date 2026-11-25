use super::*;
use crate::decode::decode;

/// The mask that holds the GPRs `gprs`.
fn mask(gprs: &[u8]) -> u32 {
    gprs.iter().fold(0, |mask, gpr| mask | 1 << gpr)
}

/// The privileged word `raw`, after a check that decode refuses it as
/// privileged.
fn privileged(raw: u32) -> PpuInstruction {
    let decoded = PpuInstruction::PpePrivileged { raw };
    assert_eq!(decode(raw).expect("the word decodes"), decoded);
    decoded
}

/// An XFX-form SPR move with the extended opcode `xo`.
///
/// [PPC-Book1 p:9 s:1.7.8] The spr field holds the two 5-bit halves of the SPR number, low half first.
fn spr_move(xo: u32, gpr: u32, spr: u32) -> u32 {
    (31 << 26) | (gpr << 21) | ((spr & 0x1F) << 16) | ((spr >> 5) << 11) | (xo << 1)
}

#[test]
fn a_store_writes_no_gpr_and_its_update_form_writes_only_ra() {
    let stw = PpuInstruction::Stw {
        rs: 9,
        ra: 1,
        imm: 0,
    };
    let stb = PpuInstruction::Stb {
        rs: 9,
        ra: 1,
        imm: 0,
    };
    let stwx = PpuInstruction::Stwx {
        rs: 0,
        ra: 9,
        rb: 10,
    };
    let stwu = PpuInstruction::Stwu {
        rs: 9,
        ra: 1,
        imm: -16,
    };
    let stwux = PpuInstruction::Stwux {
        rs: 0,
        ra: 9,
        rb: 10,
    };
    assert_eq!(written_gprs(&stw), 0);
    assert_eq!(written_gprs(&stb), 0);
    assert_eq!(written_gprs(&stwx), 0);
    assert_eq!(written_gprs(&stwu), mask(&[1]));
    assert_eq!(written_gprs(&stwux), mask(&[9]));
}

#[test]
fn a_logical_or_shift_word_writes_only_ra() {
    let or = PpuInstruction::Or {
        ra: 3,
        rs: 9,
        rb: 10,
        rc: false,
    };
    let slw = PpuInstruction::Slw {
        ra: 4,
        rs: 9,
        rb: 10,
        rc: false,
    };
    assert_eq!(written_gprs(&or), mask(&[3]));
    assert_eq!(written_gprs(&slw), mask(&[4]));
}

#[test]
fn a_load_writes_rt_and_its_update_form_also_writes_ra() {
    let lwzx = PpuInstruction::Lwzx {
        rt: 5,
        ra: 9,
        rb: 10,
    };
    let lwzux = PpuInstruction::Lwzux {
        rt: 5,
        ra: 9,
        rb: 10,
    };
    assert_eq!(written_gprs(&lwzx), mask(&[5]));
    assert_eq!(written_gprs(&lwzux), mask(&[5, 9]));
}

#[test]
fn an_update_form_load_with_ra_equal_to_rt_writes_rt() {
    // [CBE-Handbook p:254 s:9.5.9] A load or store with update whose RA is 0 or RT takes an illegal instruction interrupt.
    // The error handler can emulate the load, which writes RT.
    let lwzux_r5_r5_r10: u32 = (31 << 26) | (5 << 21) | (5 << 16) | (10 << 11) | (55 << 1);
    let decoded = decode(lwzux_r5_r5_r10).expect("the word decodes");
    assert_eq!(
        decoded,
        PpuInstruction::Lwzux {
            rt: 5,
            ra: 5,
            rb: 10,
        }
    );
    assert_eq!(written_gprs(&decoded), mask(&[5]));
}

#[test]
fn a_privileged_mtspr_writes_no_gpr_and_a_privileged_mfspr_writes_rt() {
    // [PPC-Book3 p:21 s:3.4.2] Figure 18 gives SPRG0 the number 272 and marks it privileged.
    let sprg0 = 272;
    let mtspr = privileged(spr_move(467, 9, sprg0));
    let mfspr = privileged(spr_move(339, 9, sprg0));
    assert_eq!(written_gprs(&mtspr), 0);
    assert_eq!(written_gprs(&mfspr), mask(&[9]));
}

#[test]
fn mfmsr_slbmfev_and_slbmfee_write_only_rt() {
    // [PPC-Book3 p:23 s:3.4] mfmsr places the MSR into RT.
    // [PPC-Book3 p:52 s:4.11] slbmfev and slbmfee place fields of the SLB entry that RB selects into RT.
    let mfmsr_r6 = privileged((31 << 26) | (6 << 21) | (83 << 1));
    let slbmfev_r6_r7 = privileged((31 << 26) | (6 << 21) | (7 << 11) | (851 << 1));
    let slbmfee_r6_r7 = privileged((31 << 26) | (6 << 21) | (7 << 11) | (915 << 1));
    assert_eq!(written_gprs(&mfmsr_r6), mask(&[6]));
    assert_eq!(written_gprs(&slbmfev_r6_r7), mask(&[6]));
    assert_eq!(written_gprs(&slbmfee_r6_r7), mask(&[6]));
}

#[test]
fn lq_writes_rt_and_the_next_gpr() {
    let lq_r4_0_r1 = privileged((56 << 26) | (4 << 21) | (1 << 16));
    assert_eq!(written_gprs(&lq_r4_0_r1), mask(&[4, 5]));
}

#[test]
fn an_illegal_lq_with_rt_equal_to_ra_writes_rt_and_the_next_gpr() {
    // [PPC-Book3 p:92 s:8.4] An lq with RT = RA invokes the illegal instruction error handler.
    let lq_r4_0_r4 = (56 << 26) | (4 << 21) | (4 << 16);
    let decoded = PpuInstruction::PpeIllegal { raw: lq_r4_0_r4 };
    assert_eq!(decode(lq_r4_0_r4).expect("the word decodes"), decoded);
    assert_eq!(written_gprs(&decoded), mask(&[4, 5]));
}

#[test]
fn lswi_writes_one_gpr_for_each_four_bytes_with_r0_after_r31() {
    let nine_bytes = PpuInstruction::Lswi {
        rt: 30,
        ra: 3,
        nb: 9,
    };
    let thirty_two_bytes = PpuInstruction::Lswi {
        rt: 8,
        ra: 3,
        nb: 0,
    };
    assert_eq!(written_gprs(&nine_bytes), mask(&[30, 31, 0]));
    assert_eq!(
        written_gprs(&thirty_two_bytes),
        mask(&[8, 9, 10, 11, 12, 13, 14, 15])
    );
}

#[test]
fn lmw_writes_rt_through_r31() {
    let lmw = PpuInstruction::Lmw {
        rt: 29,
        ra: 1,
        imm: 0,
    };
    assert_eq!(written_gprs(&lmw), mask(&[29, 30, 31]));
}

#[test]
fn sc_and_lswx_write_every_gpr() {
    let lswx = PpuInstruction::Lswx {
        rt: 3,
        ra: 4,
        rb: 5,
    };
    assert_eq!(written_gprs(&PpuInstruction::Sc { lev: 1 }), u32::MAX);
    assert_eq!(written_gprs(&lswx), u32::MAX);
}

#[test]
fn an_illegal_word_writes_each_gpr_its_register_fields_name() {
    // [PPC-Book3 p:96 s:9.1] mfsr RT,SR holds SR in bits 12:15, so with SR = 3 the RA field reads 3.
    let mfsr_r5_3 = (31 << 26) | (5 << 21) | (3 << 16) | (595 << 1);
    let decoded = PpuInstruction::PpeIllegal { raw: mfsr_r5_3 };
    assert_eq!(decode(mfsr_r5_3).expect("the word decodes"), decoded);
    assert_eq!(written_gprs(&decoded), mask(&[3, 5]));
}

#[test]
fn an_illegal_mfspr_writes_rt_and_each_gpr_its_register_fields_name() {
    // [CBE-Handbook p:254 s:9.5.9] An mfspr that selects a reserved SPR takes an illegal instruction interrupt.
    // [PPC-Book1 p:82 s:3.3.13] mfspr places the SPR into RT.
    // The PPE defines no SPR 2, and the low half of its spr field reads as RA = 2.
    let mfspr_r9_2 = spr_move(339, 9, 2);
    let decoded = PpuInstruction::PpeIllegal { raw: mfspr_r9_2 };
    assert_eq!(decode(mfspr_r9_2).expect("the word decodes"), decoded);
    assert_eq!(written_gprs(&decoded), mask(&[2, 9]));
}
