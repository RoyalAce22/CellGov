use super::*;
use crate::{answered_numbers, Hypervisor};
use cellgov_event::UnitId;
use cellgov_mem::lanes::{contribution, LaneIndex};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's idle thread
/// [OFW-4.93 lv2_kernel a:0x8000000000265184].
const IDLE_SC: u64 = 0x8000_0000_0026_5184;

/// The call from hardware thread `caller` in mode `mode`.
fn request(caller: Option<u8>, mode: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[0] = mode;
    HypercallRequest {
        unit: UnitId::new(1),
        number: PAUSE,
        inputs,
        pc: IDLE_SC,
        lr: 0,
        thread: caller,
        problem_state: false,
        tick: GuestTicks::new(26_159),
    }
}

fn paused_answer(thread: u8, decrementer_wakes: bool) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    answer.push_change(InterruptChange::ThreadPaused(PausedThread {
        thread,
        decrementer_wakes,
    }));
    answer.pauses_caller = true;
    HypercallDisposition::Answered(answer)
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: PAUSE,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

#[test]
fn mode_0_pauses_the_caller_with_its_decrementer_wake_and_answers_r3_0() {
    for thread in 0..2u8 {
        let r = request(Some(thread), PAUSE_MODE_DECREMENTER_WAKES);
        assert_eq!(
            pause(&r, Some("4.93")),
            paused_answer(thread, true),
            "thread {thread}"
        );
    }
}

#[test]
fn mode_1_pauses_the_caller_with_no_decrementer_wake() {
    let r = request(Some(1), PAUSE_MODE_DECREMENTER_IGNORED);
    assert_eq!(pause(&r, Some("4.93")), paused_answer(1, false));
}

#[test]
fn the_answer_ignores_r4_to_r10() {
    let mut r = request(Some(0), 0);
    r.inputs[1..].copy_from_slice(&[4, 5, 6, 7, 8, 9, 10]);
    assert_eq!(pause(&r, Some("4.93")), paused_answer(0, true));
}

#[test]
fn a_mode_other_than_0_and_1_gets_lv1s_illegal_parameter_value_and_no_pause() {
    for mode in [2, 3, 0x1_0000_0000, 0x1_0000_0001, u64::MAX] {
        let r = request(Some(0), mode);
        let disposition = pause(&r, Some("4.93"));
        assert!(
            matches!(&disposition, HypercallDisposition::Answered(a) if !a.pauses_caller),
            "{mode:#x}: {disposition:?}"
        );
        assert_eq!(
            disposition,
            HypercallDisposition::Answered(HypercallAnswer::lv1_status(PAUSE_MODE_R3, [0; 7])),
            "{mode:#x}"
        );
    }
}

#[test]
fn a_pause_from_a_unit_with_no_hardware_thread_is_a_refusal() {
    let r = request(None, 0);
    assert_eq!(
        pause(&r, Some("4.93")),
        refused(&r, "a pause from a unit that names no hardware thread")
    );
}

#[test]
fn a_pause_at_a_firmware_whose_handler_is_not_read_is_a_refusal() {
    for mode in [PAUSE_MODE_DECREMENTER_WAKES, 2] {
        let r = request(Some(0), mode);
        for firmware in [None, Some("4.92"), Some("1.50")] {
            assert_eq!(
                pause(&r, firmware),
                refused(&r, "a pause at a firmware whose lv1 handler is not read"),
                "{firmware:?} mode {mode}"
            );
        }
    }
}

#[test]
fn the_hypervisor_answers_9_and_keeps_each_paused_thread() {
    assert!(answered_numbers().contains(&PAUSE));
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let empty = hv.sync_term();
    hv.dispatch(&request(Some(1), 0));
    hv.dispatch(&request(Some(0), 1));
    let paused: Vec<_> = hv.groups().interrupt().paused().threads().collect();
    assert_eq!(
        paused,
        [
            PausedThread {
                thread: 0,
                decrementer_wakes: false,
            },
            PausedThread {
                thread: 1,
                decrementer_wakes: true,
            },
        ]
    );
    assert_eq!(hv.groups().interrupt().paused().thread(1), Some(paused[1]));
    assert_ne!(hv.sync_term(), empty);
}

#[test]
fn an_illegal_mode_leaves_the_hypervisor_unchanged() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let before = hv.clone();
    hv.dispatch(&request(Some(0), 2));
    assert_eq!(hv, before);
}

#[test]
fn a_paused_thread_state_with_no_thread_adds_no_term() {
    assert_eq!(PausedThreadState::default().sync_term(), 0);
}

/// The list holds every nonzero lane of a state with thread 0 paused in
/// mode 0 and thread 1 in mode 1, at the fixed lane source 145.
#[test]
fn paused_thread_sync_term_lane_vector_golden() {
    let mut state = PausedThreadState::default();
    state.pause(PausedThread {
        thread: 0,
        decrementer_wakes: true,
    });
    state.pause(PausedThread {
        thread: 1,
        decrementer_wakes: false,
    });
    let lane = |object, field, value| contribution(LaneIndex::new(145, object, field, 0), value);
    let lanes = [lane(0, 0, 1), lane(0, 2, 1), lane(1, 0, 1), lane(1, 1, 1)];
    let sum = lanes.into_iter().fold(0u128, u128::wrapping_add);
    assert_eq!(state.sync_term(), sum);
}
