//! `sys_event_flag_wait` (85) and `sys_event_flag_trywait` (86) answer
//! the `lv2_event_flag_check_order` probe's twelve calls as the
//! reference console does: the same return code and result word for
//! each, read from its committed capture.

use std::path::Path;

use super::*;
use crate::host::test_support::{
    extract_write_u32, fake_runtime_with_valid_sync_attr, seed_primary_ppu, VALID_SYNC_ATTR_PTR,
};
use crate::request::Lv2Request;

/// The frame the console wrote, under the reference profile. The
/// `ps3_captures` guard in `cellgov_compare` holds it to its
/// observation and provenance.
const FRAME: &str =
    "../../tests/micro/lv2_event_flag_check_order/ps3/cech20-cex-493/cgov_frame.bin";

/// The word the probe stores in the result before each call.
const RESULT_FILL: u64 = 0xDEAD_BEEF_CAFE_F00D;

const RESULT_PTR: u32 = 0x300;

#[derive(Clone, Copy)]
enum IdKind {
    Zero,
    Junk,
    Live,
}

/// The probe's cases, in the order `ppu/main.c` runs them.
const CASES: [(IdKind, u32); 6] = [
    (IdKind::Zero, 0),
    (IdKind::Junk, 0),
    (IdKind::Zero, 0x1),
    (IdKind::Zero, 0x3),
    (IdKind::Live, 0),
    (IdKind::Live, 0x3),
];

/// The frame's payload: the bytes after the `CGOV` magic and the
/// big-endian length.
fn console_payload() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(FRAME);
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let len = u32::from_be_bytes(frame[4..8].try_into().expect("4 bytes")) as usize;
    assert_eq!(frame.len(), 8 + len, "one whole frame");
    frame[8..].to_vec()
}

/// The console's `(return code, result word)` for case `index` of the
/// payload's twelve records.
fn console_case(payload: &[u8], index: usize) -> (u64, u64) {
    let at = 8 + 16 * index;
    let rc = u32::from_be_bytes(payload[at..at + 4].try_into().expect("4 bytes"));
    let word = u64::from_be_bytes(payload[at + 8..at + 16].try_into().expect("8 bytes"));
    (u64::from(rc), word)
}

/// CellGov's `(return code, result word)` for one call.
fn cellgov_case(dispatch: &Lv2Dispatch) -> (u64, u64) {
    let Lv2Dispatch::Immediate { code, effects } = dispatch else {
        panic!("expected Immediate, got {dispatch:?}");
    };
    let word = match effects.as_slice() {
        [] => RESULT_FILL,
        [cellgov_effects::Effect::SharedWriteIntent { range, bytes, .. }] => {
            assert_eq!(range.start().raw(), u64::from(RESULT_PTR));
            u64::from_be_bytes(bytes.bytes().try_into().expect("8 bytes"))
        }
        other => panic!("expected one result write, got {other:?}"),
    };
    (*code, word)
}

/// Run every case of one syscall against a fresh host holding one flag.
fn run_cases(trywait: bool) -> Vec<(u64, u64)> {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            init: 0,
        },
        src,
        &rt,
    );
    let live = match &created {
        Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    CASES
        .iter()
        .map(|&(kind, mode)| {
            let id = match kind {
                IdKind::Zero => 0,
                IdKind::Junk => 0xDEAD_BEEF,
                IdKind::Live => live,
            };
            let request = if trywait {
                Lv2Request::EventFlagTryWait {
                    id,
                    bits: 1,
                    mode,
                    result_ptr: RESULT_PTR,
                }
            } else {
                Lv2Request::EventFlagWait {
                    id,
                    bits: 1,
                    mode,
                    result_ptr: RESULT_PTR,
                    timeout: 0,
                }
            };
            cellgov_case(&host.dispatch(request, src, &rt))
        })
        .collect()
}

fn assert_matches_console(trywait: bool, first_record: usize) {
    let payload = console_payload();
    assert_eq!(&payload[0..8], &[0; 8], "the probe ran every step");
    let cellgov = run_cases(trywait);
    for (i, got) in cellgov.iter().enumerate() {
        assert_eq!(
            *got,
            console_case(&payload, first_record + i),
            "case {i}: (return code, result word)"
        );
    }
}

#[test]
fn event_flag_wait_checks_the_id_before_the_mode_as_the_console_does() {
    assert_matches_console(false, 0);
}

#[test]
fn event_flag_trywait_checks_the_id_before_the_mode_as_the_console_does() {
    assert_matches_console(true, CASES.len());
}
