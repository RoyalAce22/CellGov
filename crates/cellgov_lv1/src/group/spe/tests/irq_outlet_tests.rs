use super::*;
use crate::group::spe::LogicalSpe;
use crate::{answered_numbers, Hypervisor};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{CONSTRUCT_LOGICAL_SPE, CONSTRUCT_VIRTUAL_ADDRESS_SPACE};
use cellgov_time::GuestTicks;

/// The spe id of the 4.93 kernel's boot SPE in the capture
/// `lv2_logical_spe_object`.
const BOOT_SPE_ID: u64 = 11;

/// The `sc` of the 4.93 kernel's calls for the boot SPE
/// [OFW-4.93 lv2_kernel a:0x800000000004f1f4].
const BOOT_SC: u64 = 0x8000_0000_0004_f1f4;

/// The kernel's call for the class `class` of the SPE `spe_id`, with r3
/// and r4 as it sets them
/// [OFW-4.93 lv2_kernel a:0x800000000004f1e8-0x800000000004f1ec].
fn request(spe_id: u64, class: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number: GET_SPE_IRQ_OUTLET,
        inputs: [spe_id, class, 0, 0, 0, 0, 0, 0],
        pc: BOOT_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(63_178),
    }
}

/// A state that holds one SPE with the id `id`.
fn spes(id: u64) -> LogicalSpeState {
    let mut spes = LogicalSpeState::default();
    spes.construct(LogicalSpe {
        id,
        spe_type: 1,
        vas_id: 2,
        area: 0x0000_4c00_0028_0000,
        shadow: 0x0000_3000_0000_6000,
        shadow_real: 0x1100_0000,
    });
    spes
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: GET_SPE_IRQ_OUTLET,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

#[test]
fn each_class_of_the_boot_spe_gets_its_outlet_id_with_r5_to_r10_cleared() {
    let outlets = game_os_spe_by_id("4.93", BOOT_SPE_ID)
        .expect("the boot SPE's ids")
        .outlets;
    assert_eq!(outlets.len(), SPE_IRQ_CLASSES);
    for (class, outlet) in (0u64..).zip(outlets) {
        let d = irq_outlet(
            &request(BOOT_SPE_ID, class),
            Some("4.93"),
            &spes(BOOT_SPE_ID),
        );
        let HypercallDisposition::Answered(answer) = d else {
            panic!("class {class}: {d:?}");
        };
        assert_eq!(answer.r3, 0, "class {class}: r3");
        assert_eq!(
            answer.outputs(),
            Some([outlet, 0, 0, 0, 0, 0, 0]),
            "class {class}: r4 to r10"
        );
        assert!(answer.changes.is_empty(), "class {class}: no state change");
    }
}

#[test]
fn a_class_above_2_is_refused() {
    let what = "an SPE irq outlet call for a class above 2, whose r4 lv1 leaves as a stack value";
    for class in [3, 4, u64::from(u32::MAX) + 1, u64::MAX] {
        let r = request(BOOT_SPE_ID, class);
        assert_eq!(
            irq_outlet(&r, Some("4.93"), &spes(BOOT_SPE_ID)),
            refused(&r, what),
            "class {class}"
        );
    }
}

/// lv1 checks the class before it looks up the SPE
/// [OFW-4.93 lv1 a:0x2FC738-0x2FC768].
#[test]
fn the_class_check_comes_before_the_spe_lookup() {
    let r = request(BOOT_SPE_ID + 1, 3);
    assert_eq!(
        irq_outlet(&r, Some("4.93"), &spes(BOOT_SPE_ID)),
        refused(
            &r,
            "an SPE irq outlet call for a class above 2, whose r4 lv1 leaves as a stack value"
        )
    );
}

/// The ids 3 and 2 name an event receive port and the boot address
/// space, which are objects of other classes.
#[test]
fn an_spe_id_that_names_no_constructed_spe_is_refused() {
    let what = "an SPE irq outlet call on an spe id that the hypervisor did not construct";
    for id in [BOOT_SPE_ID + 1, 3, 2, 0] {
        let r = request(id, 0);
        assert_eq!(
            irq_outlet(&r, Some("4.93"), &spes(BOOT_SPE_ID)),
            refused(&r, what),
            "spe id {id}"
        );
    }
    let r = request(BOOT_SPE_ID, 0);
    assert_eq!(
        irq_outlet(&r, Some("4.93"), &LogicalSpeState::default()),
        refused(&r, what)
    );
}

#[test]
fn an_spe_whose_outlet_ids_no_source_gives_is_refused() {
    let r = request(20, 1);
    assert_eq!(
        irq_outlet(&r, Some("4.93"), &spes(20)),
        refused(
            &r,
            "an SPE irq outlet call on an SPE whose outlet ids no source gives"
        )
    );
}

#[test]
fn a_firmware_whose_handler_is_not_read_is_refused() {
    let what = "an SPE irq outlet call at a firmware whose lv1 handler is not read";
    let r = request(BOOT_SPE_ID, 0);
    for firmware in [None, Some("4.92"), Some("3.55")] {
        assert_eq!(
            irq_outlet(&r, firmware, &spes(BOOT_SPE_ID)),
            refused(&r, what),
            "{firmware:?}"
        );
    }
}

#[test]
fn the_hypervisor_answers_78_after_the_boot_construct_and_keeps_its_state() {
    assert!(answered_numbers().contains(&GET_SPE_IRQ_OUTLET));
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let mut vas = request(0, 0);
    vas.number = CONSTRUCT_VIRTUAL_ADDRESS_SPACE;
    vas.inputs = [0, 2, 0x1014_0000_0000_0000, 0, 0, 0, 0, 0];
    let mut spe = request(0, 0);
    spe.number = CONSTRUCT_LOGICAL_SPE;
    spe.inputs = [12, 12, 12, 12, 12, 2, 1, 0];
    spe.pc = 0x8000_0000_0004_f188;
    for setup in [vas, spe] {
        assert!(
            matches!(hv.dispatch(&setup), HypercallDisposition::Answered(_)),
            "{:?}",
            hv.refusal()
        );
    }
    let before = hv.sync_term();
    let HypercallDisposition::Answered(answer) = hv.dispatch(&request(BOOT_SPE_ID, 2)) else {
        panic!("{:?}", hv.refusal());
    };
    let outlets = game_os_spe_by_id("4.93", BOOT_SPE_ID)
        .expect("the boot SPE's ids")
        .outlets;
    assert_eq!(answer.outputs().map(|o| o[0]), Some(outlets[2]));
    assert_eq!(hv.sync_term(), before);
}
