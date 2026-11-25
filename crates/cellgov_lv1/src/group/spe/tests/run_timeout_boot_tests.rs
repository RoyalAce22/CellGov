//! The hypervisor answers the 4.93 kernel's boot call of hypercall 138
//! after its construct of the boot SPE, with the spe id that the
//! reference console's kernel holds.

use super::*;
use crate::{answered_numbers, Hypervisor};
use cellgov_event::UnitId;
use cellgov_ps3_abi::hw::ppu::CELL_PPU_TIMEBASE_HZ;
use cellgov_ps3_abi::lv1::call::{CONSTRUCT_LOGICAL_SPE, CONSTRUCT_VIRTUAL_ADDRESS_SPACE};
use cellgov_ps3_abi::lv1::spe::game_os_spe_call;
use cellgov_time::GuestTicks;

/// A thread 0 call of hypercall `number` at the `sc` address `pc`.
fn call(number: u64, pc: u64, inputs: [u64; 8]) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs,
        pc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

/// The spe id of the kernel's boot construct, which the capture
/// `lv2_logical_spe_object` reads.
fn boot_spe_id() -> u64 {
    game_os_spe_call("4.93", 0x8000_0000_0004_f188)
        .expect("the boot SPE's ids")
        .id
}

/// The kernel's boot calls before hypercall 138 that make the SPE: the
/// address space, then the construct
/// [OFW-4.93 lv2_kernel a:0x800000000004f188].
fn boot_setup() -> [HypercallRequest; 2] {
    [
        call(
            CONSTRUCT_VIRTUAL_ADDRESS_SPACE,
            0,
            [0, 2, 0x1014_0000_0000_0000, 0, 0, 0, 0, 0],
        ),
        call(
            CONSTRUCT_LOGICAL_SPE,
            0x8000_0000_0004_f188,
            [12, 12, 12, 12, 12, 2, 1, 0],
        ),
    ]
}

/// The kernel's call with the spe id in r3 and 120 times the time base
/// frequency in r4
/// [OFW-4.93 lv2_kernel a:0x800000000004f260-0x800000000004f278]. The
/// captured `sys.flash.ext` value 0xFE selects that limit; see
/// `tests/micro/lv1_repository_flash`.
fn boot_call(limit: u64) -> HypercallRequest {
    call(
        LOGICAL_SPU_SET_RUN_TIMEOUT,
        0x8000_0000_0004_f278,
        [boot_spe_id(), limit, 0, 0, 0, 0, 0, 0],
    )
}

#[test]
fn the_hypervisor_answers_the_boot_call_and_keeps_the_limit() {
    assert!(answered_numbers().contains(&LOGICAL_SPU_SET_RUN_TIMEOUT));
    let limit = 120 * CELL_PPU_TIMEBASE_HZ;
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    for setup in boot_setup() {
        assert!(
            matches!(hv.dispatch(&setup), HypercallDisposition::Answered(_)),
            "hypercall {}: {:?}",
            setup.number,
            hv.refusal()
        );
    }
    let before = hv.sync_term();
    let HypercallDisposition::Answered(answer) = hv.dispatch(&boot_call(limit)) else {
        panic!("{:?}", hv.refusal());
    };
    assert_eq!(answer.r3, 0);
    assert_eq!(answer.outputs(), Some([0; 7]));
    let held: Vec<RunTimeout> = hv
        .groups()
        .spe()
        .timeouts()
        .timeouts()
        .values()
        .copied()
        .collect();
    assert_eq!(
        held,
        [RunTimeout {
            spe: boot_spe_id(),
            limit,
        }]
    );
    assert_ne!(hv.sync_term(), before);
    assert_eq!(hv.refusal(), None);
}

/// The limit 0, which the kernel passes when `sys.flash.ext` bit 1 is
/// clear and `sys.flash.boot` is 1
/// [OFW-4.93 lv2_kernel a:0x800000000004f23c-0x800000000004f25c], leaves
/// the state as the construct left it.
#[test]
fn the_limit_0_answers_success_and_leaves_the_state_of_the_construct() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    for setup in boot_setup() {
        assert!(matches!(
            hv.dispatch(&setup),
            HypercallDisposition::Answered(_)
        ));
    }
    let before = hv.sync_term();
    let HypercallDisposition::Answered(answer) = hv.dispatch(&boot_call(0)) else {
        panic!("{:?}", hv.refusal());
    };
    assert_eq!(answer.r3, 0);
    assert!(hv.groups().spe().timeouts().timeouts().is_empty());
    assert_eq!(hv.sync_term(), before);
}
