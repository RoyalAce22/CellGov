use super::*;
use crate::{AddressSpace, AddressSpaceChange, Hypervisor, SHADOW_STORAGE_BASE};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::CONSTRUCT_VIRTUAL_ADDRESS_SPACE;
use cellgov_time::GuestTicks;

/// The vas id of the 4.93 kernel's boot address space, which the console
/// capture `lv2_vas_boot_record` reads.
const VAS_ID: u64 = 2;

/// The `sc` of the 4.93 kernel's boot call
/// [OFW-4.93 lv2_kernel a:0x800000000004f188].
const BOOT_SC: u64 = 0x8000_0000_0004_f188;

/// The spe id of the boot call in the capture `lv2_logical_spe_object`.
const BOOT_SPE_ID: u64 = 11;

/// The 4.93 kernel's boot call, with r3 to r10 as it sets them
/// [OFW-4.93 lv2_kernel a:0x800000000004f158-0x800000000004f184].
fn request() -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number: CONSTRUCT_LOGICAL_SPE,
        inputs: [12, 12, 12, 12, 12, VAS_ID, 1, 0],
        pc: BOOT_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(63_041),
    }
}

/// A memory state that holds the boot address space and whose next
/// region counter value is `next_counter`, or the boot value for `None`.
fn memory(next_counter: Option<u64>) -> MemoryGroupState {
    let mut memory = MemoryGroupState::default();
    memory.apply(MemoryChange::AddressSpace(AddressSpaceChange::Constructed(
        AddressSpace {
            vas_id: VAS_ID,
            htab_size_exp: 18,
            page_size_count: 2,
            page_shifts: 0x1014_0000_0000_0000,
            mapping: None,
        },
    )));
    if let Some(next) = next_counter {
        memory.apply(MemoryChange::CounterTaken(next - 1));
    }
    memory
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: CONSTRUCT_LOGICAL_SPE,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

fn answered(outputs: [u64; 7], spe: LogicalSpe, last_counter: u64) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, outputs);
    answer.push_change(SpeChange::Constructed(spe));
    answer.push_change(MemoryChange::CounterTaken(last_counter));
    HypercallDisposition::Answered(answer)
}

/// The area is the 512 KB region of counter value 5 and the shadow page
/// the 4 KB region of value 6, in the encoding of
/// `cellgov_ps3_abi::lv1::memory`. lv1 puts the problem state area at
/// +0x40000 and the priv2 area at +0x60000 of the area; see
/// `cellgov_ps3_abi::lv1::spe`.
#[test]
fn the_kernels_boot_call_gets_the_area_the_shadow_page_and_the_captured_id() {
    let area = 0x0000_4c00_0028_0000;
    let shadow = 0x0000_3000_0000_6000;
    let expected = answered(
        [
            0x0000_4c00_002e_0000,
            0x0000_4c00_002c_0000,
            area,
            0,
            shadow,
            BOOT_SPE_ID,
            0,
        ],
        LogicalSpe {
            id: BOOT_SPE_ID,
            spe_type: 1,
            vas_id: VAS_ID,
            area,
            shadow,
            shadow_real: SHADOW_STORAGE_BASE,
        },
        6,
    );
    assert_eq!(
        construct(
            &request(),
            Some("4.93"),
            &memory(Some(5)),
            &LogicalSpeState::default()
        ),
        expected
    );
}

#[test]
fn with_no_region_yet_the_area_and_the_shadow_page_take_the_boot_counter_values() {
    let boot = game_os_region_counter_at_boot("4.93").expect("the 4.93 boot counter");
    let area = region_address(0x8_0000, boot).expect("the area");
    let shadow = region_address(0x1000, boot + 1).expect("the shadow page");
    let expected = answered(
        [
            area + 0x6_0000,
            area + 0x4_0000,
            area,
            0,
            shadow,
            BOOT_SPE_ID,
            0,
        ],
        LogicalSpe {
            id: BOOT_SPE_ID,
            spe_type: 1,
            vas_id: VAS_ID,
            area,
            shadow,
            shadow_real: SHADOW_STORAGE_BASE,
        },
        boot + 1,
    );
    assert_eq!(
        construct(
            &request(),
            Some("4.93"),
            &memory(None),
            &LogicalSpeState::default()
        ),
        expected
    );
}

#[test]
fn the_answer_does_not_read_r10() {
    let mut r = request();
    r.inputs[7] = 0xA5A5_A5A5_A5A5_A5A5;
    let spes = LogicalSpeState::default();
    assert_eq!(
        construct(&r, Some("4.93"), &memory(Some(5)), &spes),
        construct(&request(), Some("4.93"), &memory(Some(5)), &spes)
    );
}

#[test]
fn a_call_at_a_firmware_whose_handler_is_not_read_is_refused() {
    let r = request();
    for firmware in [None, Some("3.70"), Some("4.92")] {
        assert_eq!(
            construct(&r, firmware, &memory(Some(5)), &LogicalSpeState::default()),
            refused(
                &r,
                "a logical SPE construct at a firmware whose lv1 handler is not read"
            ),
            "{firmware:?}"
        );
    }
}

/// The kernel's type 0 call [OFW-4.93 lv2_kernel a:0x80000000000356dc]
/// and type 2 call [OFW-4.93 lv2_kernel a:0x8000000000036358].
#[test]
fn the_kernels_calls_whose_spe_id_no_capture_gives_are_refused() {
    for (pc, spe_type) in [(0x8000_0000_0003_56dc, 0), (0x8000_0000_0003_6358, 2)] {
        let mut r = request();
        r.pc = pc;
        r.inputs[6] = spe_type;
        assert_eq!(
            construct(
                &r,
                Some("4.93"),
                &memory(Some(5)),
                &LogicalSpeState::default()
            ),
            refused(
                &r,
                "a logical SPE construct whose spe id no source gives for its address"
            ),
            "0x{pc:016x}"
        );
    }
}

#[test]
fn a_page_shift_or_a_type_that_the_kernels_call_does_not_pass_is_refused() {
    let mut cases = Vec::new();
    for reg in 3..=7 {
        let mut r = request();
        r.inputs[reg - 3] = 16;
        cases.push(r);
    }
    for spe_type in [0, 2] {
        let mut r = request();
        r.inputs[6] = spe_type;
        cases.push(r);
    }
    assert_eq!(cases.len(), 7);
    for r in cases {
        assert_eq!(
            construct(
                &r,
                Some("4.93"),
                &memory(Some(5)),
                &LogicalSpeState::default()
            ),
            refused(
                &r,
                "a logical SPE construct with inputs that the kernel's call does not pass"
            ),
            "{:?}",
            r.inputs
        );
    }
}

#[test]
fn a_vas_id_that_the_hypervisor_did_not_construct_is_refused() {
    let what = "a logical SPE construct on a vas id that the hypervisor did not construct";
    let mut other = request();
    other.inputs[5] = VAS_ID + 1;
    let spes = LogicalSpeState::default();
    assert_eq!(
        construct(&other, Some("4.93"), &memory(Some(5)), &spes),
        refused(&other, what)
    );
    let r = request();
    assert_eq!(
        construct(&r, Some("4.93"), &MemoryGroupState::default(), &spes),
        refused(&r, what)
    );
}

#[test]
fn a_second_boot_call_is_refused() {
    let r = request();
    let mut spes = LogicalSpeState::default();
    spes.construct(LogicalSpe {
        id: BOOT_SPE_ID,
        spe_type: 1,
        vas_id: VAS_ID,
        area: 0,
        shadow: 0,
        shadow_real: 0,
    });
    assert_eq!(
        construct(&r, Some("4.93"), &memory(Some(5)), &spes),
        refused(
            &r,
            "a logical SPE construct whose spe id the hypervisor already holds"
        )
    );
}

/// A 512 KB region's counter value has 42 - 19 = 23 bits in the
/// encoding of `cellgov_ps3_abi::lv1::memory`.
#[test]
fn a_counter_value_that_the_area_encoding_cannot_hold_is_refused() {
    let r = request();
    assert_eq!(
        construct(
            &r,
            Some("4.93"),
            &memory(Some(1 << 23)),
            &LogicalSpeState::default()
        ),
        refused(
            &r,
            "a logical SPE construct whose region addresses the encoding cannot hold"
        )
    );
}

#[test]
fn the_hypervisor_keeps_the_spe_and_moves_the_counter_past_the_shadow_page() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let mut vas = request();
    vas.number = CONSTRUCT_VIRTUAL_ADDRESS_SPACE;
    vas.inputs = [0, 2, 0x1014_0000_0000_0000, 0, 0, 0, 0, 0];
    assert!(matches!(
        hv.dispatch(&vas),
        HypercallDisposition::Answered(_)
    ));
    let before = hv.sync_term();
    let HypercallDisposition::Answered(answer) = hv.dispatch(&request()) else {
        panic!("{:?}", hv.refusal());
    };
    let boot = game_os_region_counter_at_boot("4.93").expect("the 4.93 boot counter");
    assert_eq!(
        hv.groups().spe().spes().spe(BOOT_SPE_ID),
        Some(&LogicalSpe {
            id: BOOT_SPE_ID,
            spe_type: 1,
            vas_id: VAS_ID,
            area: answer.outputs.expect("r4 to r10")[2],
            shadow: answer.outputs.expect("r4 to r10")[4],
            shadow_real: SHADOW_STORAGE_BASE,
        })
    );
    assert_eq!(hv.memory().next_counter(), Some(boot + 2));
    assert_ne!(hv.sync_term(), before);
    assert!(matches!(
        hv.dispatch(&request()),
        HypercallDisposition::Refused(_)
    ));
}
