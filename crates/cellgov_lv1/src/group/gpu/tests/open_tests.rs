use super::*;
use crate::answer::HypercallRefusal;
use crate::{answered_numbers, Hypervisor};
use cellgov_event::UnitId;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's open
/// [OFW-4.93 lv2_kernel a:0x80000000000E8F8C].
const KERNEL_SC: u64 = 0x8000_0000_000E_8F8C;

/// The open with r3 = `r3`.
fn open_call(r3: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[0] = r3;
    HypercallRequest {
        unit: UnitId::new(1),
        number: GPU_OPEN,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::new(49_263),
    }
}

fn open(settings: GpuSettings, state: &GpuState, r3: u64) -> HypercallDisposition {
    open_gpu(&open_call(r3), Some("4.93"), settings, state)
}

fn answered(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

fn opened_answer(r3: u64) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    answer.push_change(GpuChange::Opened(GpuOpen { r3 }));
    HypercallDisposition::Answered(answer)
}

fn refused(what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: GPU_OPEN,
        unit: UnitId::new(1),
        pc: KERNEL_SC,
        what,
    })
}

fn held(r3: u64) -> GpuState {
    let mut state = GpuState::default();
    state.apply(GpuChange::Opened(GpuOpen { r3 }));
    state
}

/// The open function returns 0 [OFW-4.93 lv1 a:0x20C648], and the handler
/// writes no return slot [OFW-4.93 lv1 a:0x20EBF4-0x20EC20].
#[test]
fn the_kernels_open_gets_0_and_carries_the_open() {
    let state = GpuState::default();
    assert_eq!(open(DEFAULT_SETTINGS, &state, 0), opened_answer(0));
}

/// The constructor keeps r3 at +8 [OFW-4.93 lv1 a:0x20BE2C], and no
/// check reads it.
#[test]
fn the_open_keeps_r3_and_answers_each_r3_alike() {
    let state = GpuState::default();
    for r3 in [1, 0x8000_0000_0000_0000, u64::MAX] {
        assert_eq!(open(DEFAULT_SETTINGS, &state, r3), opened_answer(r3));
    }
}

/// [OFW-4.93 lv1 a:0x20C5B0-0x20C5BC] compares the owner word with 255.
#[test]
fn an_open_while_the_partition_holds_the_gpu_gets_busy() {
    assert_eq!(
        open(DEFAULT_SETTINGS, &held(0), 0),
        answered(GPU_OPEN_BUSY_R3)
    );
    assert_eq!(
        open(DEFAULT_SETTINGS, &held(7), 9),
        answered(GPU_OPEN_BUSY_R3)
    );
}

/// [OFW-4.93 lv1 a:0x20C590-0x20C594] `cmpldi 1; ble`, before the owner
/// check.
#[test]
fn an_rsx_enable_of_1_or_less_gets_no_entry_before_the_busy_check() {
    for rsx_enable in [0, 1] {
        let settings = GpuSettings {
            rsx_enable,
            ..DEFAULT_SETTINGS
        };
        assert_eq!(open(settings, &held(0), 0), answered(GPU_OPEN_DISABLED_R3));
    }
    let settings = GpuSettings {
        rsx_enable: 2,
        ..DEFAULT_SETTINGS
    };
    assert_eq!(open(settings, &GpuState::default(), 0), opened_answer(0));
}

/// [OFW-4.93 lv1 a:0x20E964-0x20E968] answers before the open function
/// reads `sys.lv1.rsxenable`.
#[test]
fn a_boot_mode_above_3_gets_illegal_parameter_before_the_open() {
    let settings = GpuSettings {
        boot_mode: 4,
        rsx_enable: 0,
    };
    assert_eq!(open(settings, &held(0), 0), answered(GPU_OPEN_BOOT_MODE_R3));
    let settings = GpuSettings {
        boot_mode: u64::MAX,
        ..DEFAULT_SETTINGS
    };
    assert_eq!(
        open(settings, &GpuState::default(), 0),
        answered(GPU_OPEN_BOOT_MODE_R3)
    );
}

#[test]
fn a_boot_mode_of_3_is_refused_by_name() {
    let settings = GpuSettings {
        boot_mode: 3,
        ..DEFAULT_SETTINGS
    };
    let disposition = open(settings, &GpuState::default(), 0);
    assert_eq!(
        disposition,
        refused(
            "a gpu_open with a sys.boot.gos of 3, which sets an lv1 GPU state word that the \
             model does not keep"
        )
    );
    for boot_mode in [1, 2] {
        let settings = GpuSettings {
            boot_mode,
            ..DEFAULT_SETTINGS
        };
        assert_eq!(open(settings, &GpuState::default(), 0), opened_answer(0));
    }
}

/// The owner word keeps 255 [OFW-4.93 lv1 a:0x20C594]
/// [OFW-4.93 lv1 a:0x20C5C4], so the handler takes the print path
/// [OFW-4.93 lv1 a:0x20EB70-0x20EB74].
#[test]
fn a_first_open_with_an_rsx_enable_of_1_or_less_is_refused_by_name() {
    for rsx_enable in [0, 1] {
        let settings = GpuSettings {
            rsx_enable,
            ..DEFAULT_SETTINGS
        };
        assert_eq!(
            open(settings, &GpuState::default(), 0),
            refused(
                "a first gpu_open with a sys.lv1.rsxenable below 2, which stops lv1 on its \
                 print path"
            )
        );
    }
}

#[test]
fn an_unread_firmware_is_refused_by_name() {
    for firmware in [None, Some("4.92"), Some("3.70")] {
        let disposition = open_gpu(
            &open_call(0),
            firmware,
            DEFAULT_SETTINGS,
            &GpuState::default(),
        );
        assert_eq!(
            disposition,
            refused("a gpu_open at a firmware whose lv1 handler is not read"),
            "{firmware:?}"
        );
    }
}

#[test]
fn the_dispatch_answers_the_open_then_busy() {
    assert!(answered_numbers().contains(&GPU_OPEN));
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let fresh = hv.sync_term();
    assert_eq!(hv.dispatch(&open_call(0)), opened_answer(0));
    assert_eq!(hv.groups().gpu().gpu_open(), Some(GpuOpen { r3: 0 }));
    let opened = hv.sync_term();
    assert_ne!(opened, fresh);
    assert_eq!(hv.dispatch(&open_call(0)), answered(GPU_OPEN_BUSY_R3));
    assert_eq!(hv.sync_term(), opened);
}
