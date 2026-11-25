//! What each of lv1's two returns leaves in r0, r11, r12 and CR.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::dispatch::{CALLER_TSRL, HRFID_CR7_FIRMWARE, HRFID_KEEPS_CR7_FIRMWARE};
use cellgov_ps3_abi::lv1::table::HYPERCALL_TABLE_SITES;
use cellgov_time::GuestTicks;

use super::{past_the_table, table, Compare, ReturnPair, ReturnRegisters};
use crate::{HypercallDisposition, HypercallRequest, Hypervisor};

fn request(number: u64, problem_state: bool) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(3),
        number,
        inputs: [0; 8],
        pc: 0x8000_0000_0000_2000,
        lr: 0,
        thread: Some(0),
        problem_state,
        tick: GuestTicks::new(0),
    }
}

/// The registers that the answer to `request` leaves, from a hypervisor
/// for the firmware version `firmware`.
fn returned_at(firmware: &str, request: &HypercallRequest) -> ReturnRegisters {
    let mut hv = Hypervisor::new();
    hv.set_firmware(firmware);
    match hv.dispatch(request) {
        HypercallDisposition::Answered(answer) => answer.return_registers(request, Some(firmware)),
        HypercallDisposition::Refused(refusal) => panic!("{firmware}: {refusal}"),
    }
}

fn cr0_only(result: Compare) -> [Option<Compare>; 8] {
    let mut cr = [None; 8];
    cr[0] = Some(result);
    cr
}

#[test]
fn the_past_the_table_return_gives_tsrl_the_number_shifted_left_3_and_the_table_base() {
    let cases = [
        (256, 0x800, Compare::Equal),
        (0xFFFF, 0x7_FFF8, Compare::Greater),
        (0x8000_0000_0000_0000, 0, Compare::Greater),
    ];
    for (number, r11, cr0) in cases {
        assert_eq!(
            past_the_table(number, 0x37_2D08),
            ReturnRegisters {
                r0: CALLER_TSRL,
                r11_r12: ReturnPair::Values {
                    r11,
                    r12: 0x37_2D08
                },
                cr: cr0_only(cr0),
            },
            "number {number:#x}"
        );
    }
}

#[test]
fn a_4_93_hypervisor_answers_a_number_past_the_table_with_its_table_base_in_r12() {
    let returned = returned_at("4.93", &request(0x100, false));
    // The 4.93 table is at [OFW-4.93 lv1 a:0x372D08].
    assert_eq!(
        returned.r11_r12,
        ReturnPair::Values {
            r11: 0x800,
            r12: 0x37_2D08
        }
    );
}

#[test]
fn a_1_94_hypervisor_answers_a_number_past_the_table_with_0x3ac660_in_r12() {
    let returned = returned_at("1.94", &request(0x100, false));
    // The 1.94 TOC doubleword that holds the table base is at
    // [OFW-1.94 lv1 a:0x38CDB0-0x38CDB4].
    assert_eq!(
        returned.r11_r12,
        ReturnPair::Values {
            r11: 0x800,
            r12: 0x3A_C660
        }
    );
}

#[test]
fn the_table_return_gives_tsrl_the_save_restore_pair_and_the_vectors_compares() {
    let mut privileged = [None; 8];
    privileged[0] = Some(Compare::Equal);
    privileged[1] = Some(Compare::Less);
    privileged[7] = Some(Compare::Greater);
    assert_eq!(
        table(false, Some("4.93")),
        ReturnRegisters {
            r0: CALLER_TSRL,
            r11_r12: ReturnPair::SaveRestore,
            cr: privileged,
        }
    );
    let mut problem_state = privileged;
    problem_state[1] = Some(Compare::Equal);
    assert_eq!(table(true, Some("4.93")).cr, problem_state);
}

#[test]
fn the_table_return_puts_gt_in_cr7_except_where_the_vector_makes_no_call_after_the_handler() {
    assert_eq!(HRFID_CR7_FIRMWARE.len(), 4);
    for fw in HRFID_CR7_FIRMWARE {
        assert_eq!(table(false, Some(fw)).cr[7], Some(Compare::Greater), "{fw}");
    }
    assert_eq!(HRFID_KEEPS_CR7_FIRMWARE.len(), 2);
    for fw in HRFID_KEEPS_CR7_FIRMWARE {
        let cr = table(false, Some(fw)).cr;
        assert_eq!(cr[7], None, "{fw}");
        assert_eq!(cr[1], Some(Compare::Less), "{fw}");
    }
    assert_eq!(table(false, None).cr[7], Some(Compare::Greater));
    assert_eq!(table(false, Some("2.70")).cr[7], Some(Compare::Greater));
}

#[test]
fn a_handler_answer_and_a_refusal_in_the_table_return_through_hrfid() {
    // Hypercall 74 answers a privileged caller and refuses a problem-state one.
    assert_eq!(
        returned_at("4.93", &request(74, false)),
        table(false, Some("4.93"))
    );
    assert_eq!(
        returned_at("4.93", &request(74, true)),
        table(true, Some("4.93"))
    );
    // An invalid entry of the live table: the Invalid HV Call function answers.
    assert_eq!(
        returned_at("4.93", &request(14, false)),
        table(false, Some("4.93"))
    );
}

#[test]
fn every_site_answers_a_number_past_the_table_with_its_own_table_base() {
    assert_eq!(HYPERCALL_TABLE_SITES.len(), 5);
    for site in HYPERCALL_TABLE_SITES {
        let mut hv = Hypervisor::new();
        hv.set_firmware(site.firmware);
        let request = request(0x1_0000, true);
        let HypercallDisposition::Answered(answer) = hv.dispatch(&request) else {
            panic!("{}: refused", site.firmware);
        };
        assert_eq!(
            answer.return_registers(&request, Some(site.firmware)),
            past_the_table(0x1_0000, site.table_base),
            "{}",
            site.firmware
        );
    }
}
