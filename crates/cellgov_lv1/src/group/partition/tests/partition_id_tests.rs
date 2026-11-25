//! The registers `get_logical_partition_id` returns.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::GET_LOGICAL_PARTITION_ID;
use cellgov_ps3_abi::lv1::partition::LPAR_ID_GAME_OS;
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};
use cellgov_time::GuestTicks;

use crate::{HypercallDisposition, HypercallRequest, Hypervisor};

#[test]
fn the_answer_puts_the_partition_id_in_r4_and_zero_in_r5_to_r10() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let request = HypercallRequest {
        unit: UnitId::new(1),
        number: GET_LOGICAL_PARTITION_ID,
        inputs: [0xC0DE; 8],
        pc: 0x8000_0000_0001_0000,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(0),
    };
    let HypercallDisposition::Answered(answer) = hv.dispatch(&request) else {
        panic!("hypercall 74 refused");
    };
    assert_eq!(answer.r3, zero_extended(LV1_SUCCESS));
    // lv1's handler writes only the r4 slot [OFW-4.93 lv1 a:0x2EBDA8-0x2EBDC4];
    // the entry `dcbz` [OFW-4.93 lv1 a:0x0C00-0x0C08] zeroed the others.
    assert_eq!(answer.outputs(), Some([LPAR_ID_GAME_OS, 0, 0, 0, 0, 0, 0]));
}
