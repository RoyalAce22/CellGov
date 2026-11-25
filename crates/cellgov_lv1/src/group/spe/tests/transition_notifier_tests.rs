use super::*;
use crate::group::interrupt::event_port::EventReceivePort;
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

/// The port id of the kernel's boot SPE port call
/// [OFW-4.93 lv2_kernel a:0x800000000004f218], which the same capture
/// reads at +88 of the SPE object.
fn boot_port_id() -> u64 {
    game_os_port_id("4.93", 0, 0x8000_0000_0004_f218).expect("the boot SPE's port id")
}

/// The mask of the kernel's boot call
/// [OFW-4.93 lv2_kernel a:0x800000000004f22c].
const BOOT_MASK: u64 = 1;

/// The kernel's call with r3 to r5 as the 4.93 kernel's boot call sets
/// them [OFW-4.93 lv2_kernel a:0x800000000004f228-0x800000000004f238].
fn request(spe_id: u64, mask: u64, port: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number: SET_SPE_TRANSITION_NOTIFIER,
        inputs: [spe_id, mask, port, 0, 0, 0, 0, 0],
        pc: 0x8000_0000_0004_f238,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(63_185),
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

/// A state that holds the port `id`.
fn ports(id: u64) -> EventReceivePortState {
    let mut ports = EventReceivePortState::default();
    ports.construct(EventReceivePort::new(id));
    ports
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: SET_SPE_TRANSITION_NOTIFIER,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

fn answered(r3: u64, changes: &[SpeChange]) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(r3, []);
    for &change in changes {
        answer.push_change(change);
    }
    HypercallDisposition::Answered(answer)
}

#[test]
fn the_boot_call_keeps_the_notifier_with_r3_to_r10_cleared() {
    let d = set_notifier(
        &request(boot_spe_id(), BOOT_MASK, boot_port_id()),
        Some("4.93"),
        &spes(boot_spe_id()),
        &ports(boot_port_id()),
    );
    let HypercallDisposition::Answered(answer) = &d else {
        panic!("{d:?}");
    };
    assert_eq!(answer.r3, 0);
    assert_eq!(answer.outputs(), Some([0; 7]));
    assert_eq!(
        d,
        answered(
            0,
            &[SpeChange::NotifierSet(TransitionNotifier {
                spe: boot_spe_id(),
                port: boot_port_id(),
                mask: BOOT_MASK,
            })]
        )
    );
}

/// The worker's r5 = 0 path passes the mask 0 to the method
/// [OFW-4.93 lv1 a:0x2FCF38-0x2FCF64].
#[test]
fn port_0_removes_the_notifier_whatever_the_mask() {
    for mask in [0, BOOT_MASK, 0b1000, u64::MAX] {
        assert_eq!(
            set_notifier(
                &request(boot_spe_id(), mask, 0),
                Some("4.93"),
                &spes(boot_spe_id()),
                &EventReceivePortState::default(),
            ),
            answered(0, &[SpeChange::NotifierCleared(boot_spe_id())]),
            "mask {mask:#x}"
        );
    }
}

#[test]
fn port_0_on_an_spe_of_another_type_removes_the_notifier() {
    let mut held = LogicalSpeState::default();
    held.construct(spe(boot_spe_id(), 2));
    assert_eq!(
        set_notifier(
            &request(boot_spe_id(), 0, 0),
            Some("4.93"),
            &held,
            &EventReceivePortState::default(),
        ),
        answered(0, &[SpeChange::NotifierCleared(boot_spe_id())])
    );
}

#[test]
fn a_mask_bit_above_2_answers_minus_17_with_r4_marked_and_keeps_no_notifier() {
    for mask in [0b1000, 0b1001, 1 << 32, 1 << 63, u64::MAX] {
        assert_eq!(
            set_notifier(
                &request(boot_spe_id(), mask, boot_port_id()),
                Some("4.93"),
                &spes(boot_spe_id()),
                &ports(boot_port_id()),
            ),
            answered(0x0000_0002_FFFF_FFEF, &[]),
            "mask {mask:#x}"
        );
    }
}

#[test]
fn each_mask_inside_the_type_1_mask_is_kept() {
    for mask in 0..=7 {
        assert_eq!(
            set_notifier(
                &request(boot_spe_id(), mask, boot_port_id()),
                Some("4.93"),
                &spes(boot_spe_id()),
                &ports(boot_port_id()),
            ),
            answered(
                0,
                &[SpeChange::NotifierSet(TransitionNotifier {
                    spe: boot_spe_id(),
                    port: boot_port_id(),
                    mask,
                })]
            ),
            "mask {mask:#x}"
        );
    }
}

/// The boot port id and the id 2 name an event receive port and the boot
/// address space, which are objects of other classes.
#[test]
fn an_spe_id_that_names_no_constructed_spe_is_refused() {
    let what = "an SPE transition notifier call on an spe id that the hypervisor did not construct";
    for id in [boot_spe_id() + 1, boot_port_id(), 2, 0] {
        let r = request(id, BOOT_MASK, boot_port_id());
        assert_eq!(
            set_notifier(
                &r,
                Some("4.93"),
                &spes(boot_spe_id()),
                &ports(boot_port_id())
            ),
            refused(&r, what),
            "spe id {id}"
        );
    }
}

/// lv1 looks up the SPE before it reads r5
/// [OFW-4.93 lv1 a:0x2FCAC0-0x2FCC5C].
#[test]
fn the_spe_lookup_comes_before_the_port_and_the_mask() {
    let what = "an SPE transition notifier call on an spe id that the hypervisor did not construct";
    for (mask, port) in [(BOOT_MASK, 0), (BOOT_MASK, 17), (u64::MAX, boot_port_id())] {
        let r = request(boot_spe_id() + 1, mask, port);
        assert_eq!(
            set_notifier(
                &r,
                Some("4.93"),
                &spes(boot_spe_id()),
                &ports(boot_port_id())
            ),
            refused(&r, what),
            "mask {mask:#x}, port {port}"
        );
    }
}

/// The first id names the class 0 irq outlet of the boot SPE, and 12
/// names an object that no read gives.
#[test]
fn a_port_id_that_names_no_constructed_port_is_refused() {
    let what = "an SPE transition notifier call on a port id that names no event receive port \
                the hypervisor constructed";
    let outlet_0 = boot_spe().outlets[0];
    for port in [outlet_0, 12, boot_spe_id(), boot_port_id() + 1, u64::MAX] {
        let r = request(boot_spe_id(), BOOT_MASK, port);
        assert_eq!(
            set_notifier(
                &r,
                Some("4.93"),
                &spes(boot_spe_id()),
                &ports(boot_port_id())
            ),
            refused(&r, what),
            "port {port}"
        );
    }
}

/// lv1 looks up the port before its method checks the mask
/// [OFW-4.93 lv1 a:0x2FCC60-0x2FCEC0] [OFW-4.93 lv1 a:0x2FA4FC-0x2FA52C].
#[test]
fn the_port_lookup_comes_before_the_mask_check() {
    let r = request(boot_spe_id(), u64::MAX, boot_port_id() + 1);
    assert_eq!(
        set_notifier(
            &r,
            Some("4.93"),
            &spes(boot_spe_id()),
            &ports(boot_port_id())
        ),
        refused(
            &r,
            "an SPE transition notifier call on a port id that names no event receive port the \
             hypervisor constructed"
        )
    );
}

#[test]
fn a_port_on_an_spe_that_is_not_type_1_is_refused() {
    let what = "an SPE transition notifier call on an SPE whose type's accepted mask no read gives";
    for spe_type in [0, 2] {
        let mut held = LogicalSpeState::default();
        held.construct(spe(boot_spe_id(), spe_type));
        let r = request(boot_spe_id(), BOOT_MASK, boot_port_id());
        assert_eq!(
            set_notifier(&r, Some("4.93"), &held, &ports(boot_port_id())),
            refused(&r, what),
            "type {spe_type}"
        );
    }
}

#[test]
fn a_firmware_whose_handler_is_not_read_is_refused() {
    let what = "an SPE transition notifier call at a firmware whose lv1 handler is not read";
    let r = request(boot_spe_id(), BOOT_MASK, boot_port_id());
    for firmware in [None, Some("4.92"), Some("3.55")] {
        assert_eq!(
            set_notifier(&r, firmware, &spes(boot_spe_id()), &ports(boot_port_id())),
            refused(&r, what),
            "{firmware:?}"
        );
    }
}
