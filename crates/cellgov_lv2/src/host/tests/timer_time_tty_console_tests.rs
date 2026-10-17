//! The time queries, timer create and destroy, and `sys_tty_write`
//! held to the `lv2_timer_time_tty` capture from the reference console.
//! Each test drives the probe's calls through dispatch and compares the
//! return code and the bytes each call wrote. The timezone value itself
//! is the console's own setting, so only its width and the writes are
//! compared.

use cellgov_effects::Effect;
use cellgov_event::UnitId;

use super::*;
use crate::host::test_support::{console_payload, extract_write_u32, FakeRuntime};
use crate::request::Lv2Request;
use crate::Lv2Dispatch;

/// Where the guest's two out-slots sit.
const SLOT_A: u32 = 0x100;
const SLOT_B: u32 = 0x108;

/// The unmapped buffer the probe passes to `sys_tty_write`.
const UNMAPPED: u32 = 0xDEAD_0000;

fn console() -> Vec<u64> {
    console_payload("lv2_timer_time_tty")
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

fn dispatch(host: &mut Lv2Host, rt: &FakeRuntime, request: Lv2Request) -> (u64, Vec<Effect>) {
    match host.dispatch(request, UnitId::new(0), rt) {
        Lv2Dispatch::Immediate { code, effects } => (code, effects),
        other => panic!("{request:?}: expected Immediate, got {other:?}"),
    }
}

/// The 8 bytes at `slot` after `effects` land on a slot pre-filled with
/// 0xEE, as the probe pre-fills it.
fn slot_after(effects: &[Effect], slot: u32) -> u64 {
    let mut bytes = [0xEEu8; 8];
    for effect in effects {
        let Effect::SharedWriteIntent {
            range, bytes: data, ..
        } = effect
        else {
            panic!("expected a shared write, got {effect:?}");
        };
        let start = range.start().raw();
        for (i, b) in data.bytes().iter().enumerate() {
            let at = start + i as u64;
            if (u64::from(slot)..u64::from(slot) + 8).contains(&at) {
                bytes[(at - u64::from(slot)) as usize] = *b;
            }
        }
    }
    u64::from_be_bytes(bytes)
}

/// Keep the width of a written word and mask its value: the console's
/// timezone is a setting, CellGov's is UTC.
fn width_of(word: u64) -> u64 {
    word.to_be_bytes()
        .iter()
        .fold(0, |acc, &b| acc << 8 | if b == 0xEE { 0xEE } else { 0 })
}

#[test]
fn get_timezone_writes_four_bytes_each_and_faults_on_a_null_pointer_as_the_console_does() {
    let words = console();
    let rt = FakeRuntime::new(0x1000);
    let mut host = Lv2Host::new();
    let tz = |timezone_ptr, summer_time_ptr| Lv2Request::TimeGetTimezone {
        timezone_ptr,
        summer_time_ptr,
    };

    let (code, effects) = dispatch(&mut host, &rt, tz(SLOT_A, SLOT_B));
    assert_eq!(code, words[8], "both pointers: rc");
    assert_eq!(
        width_of(slot_after(&effects, SLOT_A)),
        width_of(words[9]),
        "tz width"
    );
    assert_eq!(slot_after(&effects, SLOT_B), words[10], "summer time");

    let (code, effects) = dispatch(&mut host, &rt, tz(0, SLOT_B));
    assert_eq!(code, words[11], "null timezone: rc");
    assert_eq!(
        slot_after(&effects, SLOT_B),
        words[12],
        "null timezone: summer time"
    );

    let (code, effects) = dispatch(&mut host, &rt, tz(SLOT_A, 0));
    assert_eq!(code, words[13], "null summer time: rc");
    assert_eq!(
        width_of(slot_after(&effects, SLOT_A)),
        width_of(words[14]),
        "null summer time: the timezone is still written"
    );
}

#[test]
fn get_current_time_writes_two_words_and_faults_on_a_null_pointer_as_the_console_does() {
    let words = console();
    let rt = FakeRuntime::new(0x1000);
    let mut host = Lv2Host::new();
    let (code, effects) = dispatch(
        &mut host,
        &rt,
        Lv2Request::TimeGetCurrentTime {
            sec_ptr: SLOT_A,
            nsec_ptr: SLOT_B,
        },
    );
    assert_eq!(code, words[15], "rc");
    for (slot, console_word) in [(SLOT_A, words[16]), (SLOT_B, words[17])] {
        let ours = slot_after(&effects, slot);
        assert_eq!(width_of(ours), width_of(console_word), "{slot:#x}: width");
    }
    let (code, effects) = dispatch(
        &mut host,
        &rt,
        Lv2Request::TimeGetCurrentTime {
            sec_ptr: 0,
            nsec_ptr: SLOT_B,
        },
    );
    assert_eq!(code, words[18], "null sec: rc");
    assert_eq!(slot_after(&effects, SLOT_B), words[19], "null sec: nsec");
}

#[test]
fn the_timebase_frequency_is_the_consoles() {
    let words = console();
    let rt = FakeRuntime::new(0x1000);
    let mut host = Lv2Host::new();
    assert_eq!(
        dispatch(&mut host, &rt, Lv2Request::TimeGetTimebaseFrequency).0,
        words[20]
    );
}

#[test]
fn timer_destroy_refuses_an_id_it_never_created_or_already_destroyed() {
    let words = console();
    let rt = FakeRuntime::new(0x1000);
    let mut host = Lv2Host::new();
    let destroy = |id| Lv2Request::TimerDestroy { id };
    assert_eq!(
        dispatch(&mut host, &rt, destroy(0xDEAD_BEEF)).0,
        words[22],
        "unknown id"
    );
    // The console's process held the slots below its timer's.
    for _ in 0..cellgov_ps3_abi::lv2::kernel_id::slot_of(words[24] as u32) {
        host.alloc_id(0x23).expect("a free slot");
    }
    let (code, effects) = dispatch(&mut host, &rt, Lv2Request::TimerCreate { id_ptr: SLOT_A });
    assert_eq!(code, words[23], "create");
    let id = extract_write_u32(&effects[0]);
    assert_eq!(u64::from(id), words[24], "the timer id");
    // The unknown id again with a timer live: a live count alone would
    // pass the case above, where no timer is live.
    assert_eq!(
        dispatch(&mut host, &rt, destroy(0xDEAD_BEEF)).0,
        words[22],
        "unknown id with a timer live"
    );
    assert_eq!(
        dispatch(&mut host, &rt, destroy(id)).0,
        words[25],
        "destroy"
    );
    assert_eq!(
        dispatch(&mut host, &rt, destroy(id)).0,
        words[26],
        "destroy again"
    );
}

#[test]
fn tty_write_answers_as_the_console_does() {
    let words = console();
    let rt = FakeRuntime::new(0x1000);
    let mut host = Lv2Host::new();
    let write = |buf_ptr, len, nwritten_ptr| Lv2Request::TtyWrite {
        fd: 0,
        buf_ptr,
        len,
        nwritten_ptr,
    };

    let (code, effects) = dispatch(&mut host, &rt, write(UNMAPPED, 16, SLOT_A));
    assert_eq!(code, words[27], "unmapped buffer: rc");
    assert_eq!(
        u64::from(extract_write_u32(&effects[0])),
        words[28],
        "unmapped buffer: nwritten"
    );
    assert_eq!(
        dispatch(&mut host, &rt, write(SLOT_B, 4, 0)).0,
        words[29],
        "null nwritten"
    );
    let (code, effects) = dispatch(&mut host, &rt, write(SLOT_B, 0, SLOT_A));
    assert_eq!(code, words[30], "zero length: rc");
    assert_eq!(
        u64::from(extract_write_u32(&effects[0])),
        words[31],
        "zero length: nwritten"
    );
}
