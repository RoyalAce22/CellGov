use super::*;
use cellgov_event::UnitId;
use cellgov_time::GuestTicks;

fn send_request(len: u64, text: &[u8; 32]) -> HypercallRequest {
    let word = |i: usize| u64::from_be_bytes(text[8 * i..8 * i + 8].try_into().expect("8"));
    HypercallRequest {
        unit: UnitId::new(0),
        number: LOGICAL_CONSOLE_SEND,
        inputs: [1, len, word(0), word(1), word(2), word(3), 0, 0],
        pc: 0x8000_0000_0000_fcd8,
        lr: 0,
        thread: None,
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

const TEXT: &[u8; 32] = b"lv2(0): # system software ver...";

#[test]
fn a_send_gives_the_sink_the_r4_bytes_r5_holds_and_answers_the_count_in_r4() {
    let HypercallDisposition::Answered(answer) = send(&send_request(12, TEXT)) else {
        panic!("a send of 12 bytes is answered");
    };
    assert_eq!(answer.r3, 0);
    assert_eq!(
        answer.outputs(),
        Some([12, 0, 0, 0, 0, 0, 0]),
        "r4, and r5 to r10"
    );
    let write = answer.console.expect("a console write");
    assert_eq!(write.console, 1);
    assert_eq!(write.bytes(), b"lv2(0): # sy");
}

#[test]
fn a_send_of_32_bytes_takes_all_of_r5_to_r8() {
    let HypercallDisposition::Answered(answer) = send(&send_request(32, TEXT)) else {
        panic!("a send of 32 bytes is answered");
    };
    assert_eq!(answer.console.expect("a write").bytes(), TEXT);
    assert_eq!(answer.outputs().map(|o| o[0]), Some(32), "r4");
}

#[test]
fn a_send_longer_than_32_bytes_is_a_refusal() {
    for len in [33, 256, u64::MAX] {
        let r = send_request(len, TEXT);
        let d = send(&r);
        assert!(
            matches!(
                d,
                HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
                    number: LOGICAL_CONSOLE_SEND,
                    unit,
                    pc,
                    ..
                }) if unit == r.unit && pc == r.pc
            ),
            "{len}: {d:?}"
        );
    }
}

#[test]
fn construct_and_flush_succeed_with_no_output() {
    for d in [construct(), flush()] {
        assert_eq!(
            d,
            HypercallDisposition::Answered(HypercallAnswer::success([]))
        );
    }
}
