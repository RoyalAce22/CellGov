use super::*;
use crate::group::spe::LogicalSpe;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::event_port::game_os_port_id;
use cellgov_ps3_abi::lv1::spe::{game_os_spe_call, GameOsSpeCall};
use cellgov_time::GuestTicks;

/// The 4.93 kernel's boot SPE construct
/// [OFW-4.93 lv2_kernel a:0x800000000004f188], with the ids that the
/// capture `lv2_logical_spe_object` reads.
fn boot_spe() -> &'static GameOsSpeCall {
    game_os_spe_call("4.93", 0x8000_0000_0004_f188).expect("the boot SPE's ids")
}

fn boot_spe_id() -> u64 {
    boot_spe().id
}

/// The kernel's call with `spe_id`, `class` and `mask` in r3 to r5, where
/// the 4.93 kernel's call sets them
/// [OFW-4.93 lv2_kernel a:0x8000000000034c34-0x8000000000034c48].
fn request(spe_id: u64, class: u64, mask: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number: SET_SPE_INTERRUPT_MASK,
        inputs: [spe_id, class, mask, 0, 0, 0, 0, 0],
        pc: 0x8000_0000_0003_4c48,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(63_312),
    }
}

fn spe(id: u64, spe_type: u64) -> LogicalSpe {
    LogicalSpe {
        id,
        spe_type,
        vas_id: 2,
        area: 0x0000_4c00_0028_0000,
        shadow: 0x0000_3000_0000_6000,
        shadow_real: 0x1100_0000,
    }
}

/// A state that holds the type 1 SPE `id`.
fn spes(id: u64) -> LogicalSpeState {
    let mut spes = LogicalSpeState::default();
    spes.construct(spe(id, 1));
    spes
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: SET_SPE_INTERRUPT_MASK,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

fn kept(spe: u64, class: u64, mask: u64) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, []);
    answer.push_change(SpeChange::InterruptMaskSet(InterruptMask {
        spe,
        class,
        mask,
    }));
    HypercallDisposition::Answered(answer)
}

/// The kernel's first call of its boot loop passes the class 0 and the
/// mask 0 [OFW-4.93 lv2_kernel a:0x800000000004f820-0x800000000004f834].
#[test]
fn the_first_boot_call_keeps_the_mask_0_with_r3_to_r10_cleared() {
    let d = set_interrupt_mask(
        &request(boot_spe_id(), 0, 0),
        Some("4.93"),
        &spes(boot_spe_id()),
    );
    let HypercallDisposition::Answered(answer) = &d else {
        panic!("{d:?}");
    };
    assert_eq!(answer.r3, 0);
    assert_eq!(answer.outputs(), Some([0; 7]));
    assert_eq!(d, kept(boot_spe_id(), 0, 0));
}

/// lv1's method ANDs r5 with the class's doubleword of its table, which
/// is 0xFFFFFFFE_FFFFFFF7 for class 0 and has each bit set for classes 1
/// and 2 [OFW-4.93 lv1 a:0x2FA24C-0x2FA25C]
/// [OFW-4.93 lv1 a:0x329A98-0x329AAC].
#[test]
fn each_class_keeps_the_bits_of_r5_that_lv1_keeps_for_the_class() {
    let cases = [
        (0, 7, 7),
        (0, u64::MAX, 0xFFFF_FFFE_FFFF_FFF7),
        (0, 0x8, 0),
        (0, 0x0000_0001_0000_0000, 0),
        (0, 0x0000_0001_0000_000F, 7),
        (1, 3, 3),
        (1, u64::MAX, u64::MAX),
        (2, 7, 7),
        (2, u64::MAX, u64::MAX),
    ];
    assert_eq!(cases.len(), 9);
    for (class, mask, held) in cases {
        assert_eq!(
            set_interrupt_mask(
                &request(boot_spe_id(), class, mask),
                Some("4.93"),
                &spes(boot_spe_id())
            ),
            kept(boot_spe_id(), class, held),
            "class {class}, mask {mask:#x}"
        );
    }
}

/// lv1's worker checks the class before it looks up r3
/// [OFW-4.93 lv1 a:0x2FE340] [OFW-4.93 lv1 a:0x2FE36C].
#[test]
fn a_class_above_2_gets_minus_17_marked_for_r4_whatever_r3_names() {
    let class_refusal = HypercallDisposition::Answered(HypercallAnswer::lv1_status(
        SET_SPE_INTERRUPT_MASK_CLASS_R3,
        [],
    ));
    let classes = [3, 4, 0x0000_0001_0000_0000, u64::MAX];
    let ids = [boot_spe_id(), boot_spe_id() + 1, 0];
    for (class, id) in classes.into_iter().flat_map(|c| ids.map(|id| (c, id))) {
        let d = set_interrupt_mask(&request(id, class, 7), Some("4.93"), &spes(boot_spe_id()));
        assert_eq!(d, class_refusal, "class {class:#x}, spe id {id}");
        let HypercallDisposition::Answered(answer) = &d else {
            panic!("{d:?}");
        };
        assert_eq!(answer.outputs(), Some([0; 7]));
    }
}

/// lv1's handler gives only r3 to r5 to the worker
/// [OFW-4.93 lv1 a:0x2FE600-0x2FE60C].
#[test]
fn r6_to_r10_do_not_change_the_answer() {
    let mut r = request(boot_spe_id(), 1, 3);
    r.inputs[3..].copy_from_slice(&[u64::MAX, 1, 2, 3, 0x8000_0000_0000_0000]);
    assert_eq!(
        set_interrupt_mask(&r, Some("4.93"), &spes(boot_spe_id())),
        kept(boot_spe_id(), 1, 3)
    );
}

/// The boot SPE's outlet and port ids and the id 2 name objects of other
/// kinds, and 12 names an object that no read gives.
#[test]
fn an_spe_id_that_names_no_constructed_spe_is_refused() {
    let what = "an SPE interrupt mask call on an spe id that the hypervisor did not construct";
    let port = game_os_port_id("4.93", 0, 0x8000_0000_0004_f218).expect("the boot SPE's port id");
    let outlet_0 = boot_spe().outlets[0];
    for id in [boot_spe_id() + 1, outlet_0, port, 2, 0, u64::MAX] {
        let r = request(id, 0, 7);
        assert_eq!(
            set_interrupt_mask(&r, Some("4.93"), &spes(boot_spe_id())),
            refused(&r, what),
            "spe id {id}"
        );
    }
}

#[test]
fn an_spe_that_is_not_type_1_is_refused() {
    let what = "an SPE interrupt mask call on an SPE whose type's interrupt objects no read gives";
    for spe_type in [0, 2] {
        let mut held = LogicalSpeState::default();
        held.construct(spe(boot_spe_id(), spe_type));
        let r = request(boot_spe_id(), 0, 7);
        assert_eq!(
            set_interrupt_mask(&r, Some("4.93"), &held),
            refused(&r, what),
            "type {spe_type}"
        );
    }
}

#[test]
fn a_firmware_whose_handler_is_not_read_is_refused() {
    let what = "an SPE interrupt mask call at a firmware whose lv1 handler is not read";
    for class in [0, 3] {
        let r = request(boot_spe_id(), class, 7);
        for firmware in [None, Some("4.92"), Some("3.55")] {
            assert_eq!(
                set_interrupt_mask(&r, firmware, &spes(boot_spe_id())),
                refused(&r, what),
                "{firmware:?}, class {class}"
            );
        }
    }
}
