use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{
    CONFIGURE_IRQ_STATE_BITMAP, CONNECT_IRQ_PLUG_EXT, CONSTRUCT_LOGICAL_SPE,
    CONSTRUCT_VIRTUAL_ADDRESS_SPACE,
};
use cellgov_ps3_abi::lv1::spe::game_os_spe_call;
use cellgov_time::GuestTicks;

/// The bitmap of the 4.93 kernel's thread 0: +0x1C0 of its record, which
/// the capture `lv2_logical_ppe_record` places at 0x8000000000368f80.
const THREAD_0_BITMAP: u64 = 0x0036_9140;

/// The `sc` of the kernel's boot construct of its SPE
/// [OFW-4.93 lv2_kernel a:0x800000000004f188].
const SPE_CONSTRUCT_SC: u64 = 0x8000_0000_0004_f188;

fn storage() -> GuestMemory {
    GuestMemory::new(0x0037_0000)
}

fn call(number: u64, pc: u64, args: &[u64]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..args.len()].copy_from_slice(args);
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

/// A 4.93 hypervisor after the kernel's address space, the bitmap of
/// thread 0 and the boot SPE.
fn with_boot_spe() -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    for setup in [
        call(
            CONSTRUCT_VIRTUAL_ADDRESS_SPACE,
            0,
            &[0, 2, 0x1014_0000_0000_0000],
        ),
        call(CONFIGURE_IRQ_STATE_BITMAP, 0, &[1, 0, THREAD_0_BITMAP]),
        call(
            CONSTRUCT_LOGICAL_SPE,
            SPE_CONSTRUCT_SC,
            &[12, 12, 12, 12, 12, 2, 1],
        ),
    ] {
        assert!(
            matches!(
                hv.dispatch_with(&setup, &storage()),
                HypercallDisposition::Answered(_)
            ),
            "hypercall {}: {:?}",
            setup.number,
            hv.refusal()
        );
    }
    hv
}

/// lv1 reads the SPE class at +48 of the object, which is not the port
/// class, and returns the r6-marked -6
/// [OFW-4.93 lv1 a:0x2EA18C-0x2EA198] [OFW-4.93 lv1 a:0x2EA19C-0x2EA1A8].
#[test]
fn the_dispatcher_answers_a_held_spe_id_with_lv1_s_port_r3_and_changes_nothing() {
    let mut hv = with_boot_spe();
    let spe_id = game_os_spe_call("4.93", SPE_CONSTRUCT_SC)
        .expect("the boot SPE's ids")
        .id;
    let before = (hv.event_receive_ports().clone(), hv.sync_term());
    let r = call(CONNECT_IRQ_PLUG_EXT, 0, &[1, 0, 0, spe_id, 0]);
    let HypercallDisposition::Answered(answer) = hv.dispatch_with(&r, &storage()) else {
        panic!("r6 = {spe_id}: {:?}", hv.refusal());
    };
    assert_eq!(answer.r3, IRQ_PLUG_PORT_R3);
    assert_eq!(answer.outputs(), Some([0; 7]));
    assert_eq!((hv.event_receive_ports().clone(), hv.sync_term()), before);
    assert_eq!(hv.refusal(), None);
}
