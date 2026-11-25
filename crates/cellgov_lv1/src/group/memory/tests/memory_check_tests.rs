//! allocate_memory answers lv1's own r3 for each of lv1's five argument
//! checks, in lv1's order, at each firmware whose lv1 shows the checks.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::ALLOCATE_MEMORY;
use cellgov_ps3_abi::lv1::memory::{
    ALLOCATE_MEMORY_FLAGS_R3, ALLOCATE_MEMORY_MAX_SIZE, ALLOCATE_MEMORY_PAGE_SIZE_EXP_R3,
    ALLOCATE_MEMORY_R5_R3, ALLOCATE_MEMORY_SIZE_ALIGNMENT_R3, ALLOCATE_MEMORY_SIZE_TOO_LARGE_R3,
    ALLOCATE_MEMORY_WORKERS,
};
use cellgov_time::GuestTicks;

use crate::{
    HypercallAnswer, HypercallDisposition, HypercallRefusal, HypercallRequest, Hypervisor,
};

fn request(inputs: [u64; 4]) -> HypercallRequest {
    let mut all = [0; 8];
    all[..4].copy_from_slice(&inputs);
    HypercallRequest {
        unit: UnitId::new(1),
        number: ALLOCATE_MEMORY,
        inputs: all,
        pc: 0x8000_0000_0026_53c0,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(7),
    }
}

fn lv1_answer(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

fn dispatch_at(firmware: &str, inputs: [u64; 4]) -> (HypercallDisposition, Hypervisor) {
    let mut hv = Hypervisor::new();
    hv.set_firmware(firmware);
    let d = hv.dispatch(&request(inputs));
    (d, hv)
}

#[test]
fn each_failed_check_answers_lv1s_r3_and_keeps_no_region() {
    let cases: [([u64; 4], u64); 7] = [
        (
            [0x4000_0000_0000_0000, 20, 0, 0],
            ALLOCATE_MEMORY_SIZE_TOO_LARGE_R3,
        ),
        ([0x10_0000, 11, 0, 0], ALLOCATE_MEMORY_PAGE_SIZE_EXP_R3),
        ([0x10_0000, 62, 0, 0], ALLOCATE_MEMORY_PAGE_SIZE_EXP_R3),
        ([0x10_0000, 0, 0, 0], ALLOCATE_MEMORY_PAGE_SIZE_EXP_R3),
        ([0x10_8000, 16, 0, 0], ALLOCATE_MEMORY_SIZE_ALIGNMENT_R3),
        ([0x10_0000, 20, 2, 0], ALLOCATE_MEMORY_R5_R3),
        ([0x10_0000, 20, 0, 0x10], ALLOCATE_MEMORY_FLAGS_R3),
    ];
    for (inputs, r3) in cases {
        let (d, hv) = dispatch_at("4.93", inputs);
        assert_eq!(d, lv1_answer(r3), "{inputs:x?}");
        assert!(hv.memory().regions().is_empty(), "{inputs:x?}");
        assert_eq!(hv.refusal(), None, "{inputs:x?}");
    }
}

#[test]
fn the_first_failed_check_in_lv1s_order_is_the_answer() {
    let cases: [([u64; 4], u64); 4] = [
        (
            [0x4000_0000_0000_0001, 70, 5, 0xF0],
            ALLOCATE_MEMORY_SIZE_TOO_LARGE_R3,
        ),
        ([0x10_0001, 70, 5, 0xF0], ALLOCATE_MEMORY_PAGE_SIZE_EXP_R3),
        ([0x10_0001, 20, 5, 0xF0], ALLOCATE_MEMORY_SIZE_ALIGNMENT_R3),
        ([0x10_0000, 20, 5, 0xF0], ALLOCATE_MEMORY_R5_R3),
    ];
    for (inputs, r3) in cases {
        assert_eq!(dispatch_at("4.93", inputs).0, lv1_answer(r3), "{inputs:x?}");
    }
}

#[test]
fn the_largest_size_and_the_exponent_bounds_pass_lv1s_checks() {
    for inputs in [
        [0x3FFF_FFFF_FFF0_0000, 20, 0, 0],
        [0x1000, 12, 0, 0],
        [0x2000_0000_0000_0000, 61, 0, 0],
    ] {
        let (d, _) = dispatch_at("4.93", inputs);
        assert!(
            matches!(d, HypercallDisposition::Refused(_)),
            "{inputs:x?} passes lv1's checks and reaches a CellGov refusal: {d:?}"
        );
    }
}

#[test]
fn the_largest_size_passes_the_size_check_and_flag_bit_3_passes_the_flags_check() {
    assert_eq!(
        dispatch_at("4.93", [ALLOCATE_MEMORY_MAX_SIZE, 12, 0, 0]).0,
        lv1_answer(ALLOCATE_MEMORY_SIZE_ALIGNMENT_R3)
    );
    let flagged = request([0x10_0000, 20, 0, 0x8]);
    assert_eq!(
        dispatch_at("4.93", [0x10_0000, 20, 0, 0x8]).0,
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
            number: ALLOCATE_MEMORY,
            unit: flagged.unit,
            pc: flagged.pc,
            what: "an allocation with flags in the low 4 bits of r6",
        })
    );
}

#[test]
fn every_firmware_whose_worker_was_read_answers_lv1s_r3() {
    assert_eq!(ALLOCATE_MEMORY_WORKERS.len(), 5);
    for (firmware, _) in ALLOCATE_MEMORY_WORKERS {
        assert_eq!(
            dispatch_at(firmware, [0x10_0000, 20, 2, 0]).0,
            lv1_answer(ALLOCATE_MEMORY_R5_R3),
            "{firmware}"
        );
    }
}

#[test]
fn a_failed_check_is_refused_where_no_lv1_image_shows_the_checks() {
    let failing = request([0x10_0000, 11, 2, 0x10]);
    let refused = |what: &'static str| {
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
            number: ALLOCATE_MEMORY,
            unit: failing.unit,
            pc: failing.pc,
            what,
        })
    };
    let mut at_1_50 = Hypervisor::new();
    at_1_50.set_firmware("1.50");
    assert_eq!(
        at_1_50.dispatch(&failing),
        refused("an allocation at a firmware whose lv1 argument checks are not read")
    );
    assert_eq!(
        Hypervisor::new().dispatch(&failing),
        refused("an allocation with no firmware version")
    );
}
