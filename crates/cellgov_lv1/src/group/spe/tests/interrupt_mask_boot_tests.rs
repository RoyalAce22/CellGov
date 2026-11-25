//! The hypervisor answers the six calls of hypercall 61 in the 4.93
//! kernel's boot loop after the construct of its boot SPE.
//!
//! The loop runs four steps with the index 0 to 3
//! [OFW-4.93 lv2_kernel a:0x8000000000034aac-0x8000000000034ae8]. Each
//! step for the index 0 to 2 calls 61 for the class that the index names
//! twice:
//! - before the step's connect, with the mask 0;
//! - after the connect, with the mask that the step stores at +16 of its
//!   object [OFW-4.93 lv2_kernel a:0x800000000004f794-0x800000000004f7a8].
//!
//! The step for the index 3 makes no call
//! [OFW-4.93 lv2_kernel a:0x800000000004f83c-0x800000000004f854].
//!
//! The capture `lv2_boot_spe_interrupt_masks` gives the last mask of each
//! class that the console's kernel passed.

use super::*;
use crate::{answered_numbers, Hypervisor};
use cellgov_event::UnitId;
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

/// The kernel's boot calls that make the SPE: the address space, then the
/// construct [OFW-4.93 lv2_kernel a:0x800000000004f188].
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

/// The class and the mask of each call, in the loop's order:
/// - index 0: class 0 with the mask 0, then 7
///   [OFW-4.93 lv2_kernel a:0x800000000004f81c-0x800000000004f834];
/// - index 1: class 1 with the mask 0, then 3
///   [OFW-4.93 lv2_kernel a:0x800000000004f73c-0x800000000004f754];
/// - index 2: class 2 with the mask 0, then 7
///   [OFW-4.93 lv2_kernel a:0x800000000004f7e4-0x800000000004f7fc].
const BOOT_LOOP_CALLS: [(u64, u64); 6] = [(0, 0), (0, 7), (1, 0), (1, 3), (2, 0), (2, 7)];

/// The kernel's call with the spe id from +8 of its SPE object in r3
/// [OFW-4.93 lv2_kernel a:0x8000000000034c3c-0x8000000000034c48].
fn boot_call(class: u64, mask: u64) -> HypercallRequest {
    call(
        SET_SPE_INTERRUPT_MASK,
        0x8000_0000_0003_4c48,
        [boot_spe_id(), class, mask, 0, 0, 0, 0, 0],
    )
}

/// A 4.93 hypervisor after the kernel's calls that make the boot SPE.
fn booted() -> Hypervisor {
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
    hv
}

fn held(hv: &Hypervisor) -> Vec<InterruptMask> {
    hv.groups()
        .spe()
        .masks()
        .masks()
        .values()
        .copied()
        .collect()
}

/// The value of the field `field` in the capture
/// `lv2_boot_spe_interrupt_masks`. Both reads of the word must agree.
fn captured(field: &str) -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/lv2_boot_spe_interrupt_masks")
        .join("peek/cech20-cex-493/interrupt_masks.tsv");
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("address\tfield\twidth\treads\tagree\tvalue"),
        "{name}: header"
    );
    let row: Vec<&str> = lines
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .find(|row| row.get(1) == Some(&field))
        .unwrap_or_else(|| panic!("{name}: no row for {field}"));
    assert_eq!(row.len(), 6, "{name}: {field} columns");
    assert_eq!(
        (row[2], row[3], row[4]),
        ("8", "2", "yes"),
        "{name}: {field}"
    );
    u64::from_str_radix(row[5].trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("{name}: {field}: {e}"))
}

/// The console's kernel stores the mask of each call at +56 + 8 * class
/// of its SPE object
/// [OFW-4.93 lv2_kernel a:0x8000000000034c4c-0x8000000000034c60]. So its
/// words hold the last mask of each class.
#[test]
fn the_kept_masks_are_the_last_masks_that_the_console_kernel_holds() {
    assert_eq!(captured("spe_id"), boot_spe_id());
    let mut hv = booted();
    for (class, mask) in BOOT_LOOP_CALLS {
        assert!(
            matches!(
                hv.dispatch(&boot_call(class, mask)),
                HypercallDisposition::Answered(HypercallAnswer { r3: 0, .. })
            ),
            "class {class}, mask {mask}: {:?}",
            hv.refusal()
        );
    }
    let console: Vec<InterruptMask> = [0u64, 1, 2]
        .map(|class| InterruptMask {
            spe: boot_spe_id(),
            class,
            mask: captured(&format!("mask_{class}")),
        })
        .into();
    assert_eq!(held(&hv), console);
}

#[test]
fn the_hypervisor_answers_the_six_boot_loop_calls_and_keeps_the_last_mask_of_each_class() {
    assert!(answered_numbers().contains(&SET_SPE_INTERRUPT_MASK));
    let mut hv = booted();
    for (class, mask) in BOOT_LOOP_CALLS {
        let before = hv.sync_term();
        let HypercallDisposition::Answered(answer) = hv.dispatch(&boot_call(class, mask)) else {
            panic!("class {class}, mask {mask}: {:?}", hv.refusal());
        };
        assert_eq!(answer.r3, 0, "class {class}, mask {mask}");
        assert_eq!(answer.outputs(), Some([0; 7]), "class {class}, mask {mask}");
        assert_eq!(
            hv.sync_term() == before,
            mask == 0,
            "class {class}, mask {mask}"
        );
    }
    let kept = |class, mask| InterruptMask {
        spe: boot_spe_id(),
        class,
        mask,
    };
    assert_eq!(held(&hv), [kept(0, 7), kept(1, 3), kept(2, 7)]);
    assert_eq!(hv.refusal(), None);
}
