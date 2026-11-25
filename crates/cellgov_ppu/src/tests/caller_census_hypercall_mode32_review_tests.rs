use super::hypercall_entry_tests::{code, ordinals, pointers, word};
use super::hypercall_real_mode_switch_tests::{lis_sf, mtspr, ori, RFID};
use super::hypercall_tests::{b, li, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_SC_1;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_IR, SPR_SRR0, SPR_SRR1};

/// The address of a window below 4 GiB: `li r11,75; nop; sc; b .`.
const WINDOW: u64 = 0x3000;

/// The address of the window's `sc`.
const WINDOW_SC: u64 = WINDOW + 8;

/// The address of the stub that runs the `rfid`.
const STUB: u64 = 0x6000;

/// The address of the stub that builds 0x1_0000_3004.
const BUILDER: u64 = 0x7000;

/// The stub at [`BUILDER`] names 0x1_0000_3004, whose low-order 32 bits
/// are the address of the window's `nop`. Code pointers name both stubs.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` fetches from 32 zeros || SRR0 32:61 || 0b00 when SF = 0 in the new MSR.
#[test]
fn an_rfid_to_a_loaded_srr0_with_sf_at_0_enters_the_low_word_when_no_bcctr_or_bclr_exists() {
    let census = |msr: &[u32]| {
        let mut switch = vec![word(PpuInstruction::Ld {
            rt: 3,
            ra: 4,
            imm: 0,
        })];
        switch.extend_from_slice(msr);
        switch.extend([mtspr(SPR_SRR0, 3), mtspr(SPR_SRR1, 5), RFID]);
        let builder = [
            li(3, 1),
            word(PpuInstruction::Rldicr {
                ra: 3,
                rs: 3,
                sh: 32,
                me: 31,
                rc: false,
            }),
            word(PpuInstruction::Ori {
                ra: 3,
                rs: 3,
                imm: 0x3004,
            }),
            b(0),
        ];
        let segments = [
            code(STUB, &switch),
            code(BUILDER, &builder),
            code(WINDOW, &[li(11, 75), NOP, PPC_SC_1, b(0)]),
            pointers(&[STUB, BUILDER]),
        ];
        ordinals(0, &segments)
    };
    let msr_ir = [li(5, i16::try_from(MSR_IR).expect("a low MSR bit"))];
    let msr_sf_ir = [lis_sf(5), ori(5, MSR_IR)];
    assert_eq!(census(&msr_ir), [(WINDOW_SC, None)]);
    assert_eq!(census(&msr_sf_ir), [(WINDOW_SC, Some(75))]);
}
