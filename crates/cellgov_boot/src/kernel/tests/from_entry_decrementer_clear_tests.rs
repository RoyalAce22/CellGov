use super::*;

/// `lis r4, 0x8000`: a decrementer with bit 0 set, which is negative.
const LIS_R4_8000: u32 = 0x3C80_8000;

/// `mtspr 22, r4`: `mtdec r4`.
const MTDEC_R4: u32 = 0x7C96_03A6;

/// `mfspr r8, 22`: `mfdec r8`.
const MFDEC_R8: u32 = 0x7D16_02A6;

/// The pause begins with DEC negative, so the decrementer ends it at once.
/// lv1's return writes DEC = 0 [OFW-4.93 lv1 a:0x2C425C-0x2C4274], so the
/// `mfdec` after the call reads 0 and the console gets it.
#[test]
fn a_decrementer_wake_from_a_negative_decrementer_resumes_with_dec_0() {
    let mut code = vec![LIS_R4_8000, MTDEC_R4, LI_R3_0, LI_R11_PAUSE, SC_1, MFDEC_R8];
    code.extend(send_r8());
    code.push(B_SELF);
    let spin = u64::try_from(code.len()).expect("a short program") - 1;
    let run = run_kernel_from_entry(&kernel_elf(&code), FW_PAUSE, 200).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap);
    let thread0 = run.thread0.expect("thread 0's position");
    assert_eq!(thread0.pc, ENTRY + 4 * spin);
    assert_eq!(run.console, 0u64.to_be_bytes().to_vec());
}
