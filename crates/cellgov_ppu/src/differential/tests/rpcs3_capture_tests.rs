//! RPCS3 capture-trace decoding: record round trips and malformed-stream rejection.

use super::*;
use crate::state::PpuState;

fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_u64(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_state(buf: &mut Vec<u8>, state: &PpuStateSnapshot, rtime: u64, control: ControlRegisters) {
    for v in state.gpr {
        push_u64(buf, v);
    }
    for v in state.fpr {
        push_u64(buf, v);
    }
    for v in state.vr {
        buf.extend_from_slice(&v.to_be_bytes());
    }
    push_u32(buf, state.cr);
    push_u64(buf, state.lr);
    push_u64(buf, state.ctr);
    push_u64(buf, state.xer);
    if control == ControlRegisters::Recorded {
        push_u32(buf, state.vrsave);
        push_u32(buf, state.fpscr);
        push_u32(buf, state.vscr);
    }
    let raddr = state.reservation.map(|r| r.addr() as u32).unwrap_or(0);
    push_u32(buf, raddr);
    push_u64(buf, rtime);
}

/// A trace in the format version that the first record's
/// `control_registers` names: 4 for `Recorded`, 3 for `NotRecorded`.
fn synthesize_trace(records: &[CapturedRecord]) -> Vec<u8> {
    let control = records
        .first()
        .map_or(ControlRegisters::Recorded, |r| r.control_registers);
    let version = match control {
        ControlRegisters::Recorded => FORMAT_VERSION,
        ControlRegisters::NotRecorded => OLDEST_FORMAT_VERSION,
    };
    let mut buf = Vec::new();
    push_u32(&mut buf, HEADER_MAGIC);
    push_u32(&mut buf, version);
    for r in records {
        push_u32(&mut buf, RECORD_MAGIC);
        push_u64(&mut buf, r.pc);
        push_u32(&mut buf, r.raw_instruction);
        push_u32(&mut buf, r.thread_id);
        push_state(&mut buf, &r.pre_state, r.pre_reservation_rtime, control);
        push_state(&mut buf, &r.post_state, r.post_reservation_rtime, control);
        push_u64(&mut buf, r.mem_addr);
        push_u32(&mut buf, r.mem_pre.len() as u32);
        buf.extend_from_slice(&r.mem_pre);
        buf.extend_from_slice(&r.mem_post);
    }
    buf
}

fn sample_record() -> CapturedRecord {
    let mut pre = PpuStateSnapshot::zero();
    pre.gpr[3] = 0xDEAD_BEEF;
    pre.vr[5] = 0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210u128;
    pre.vrsave = 0xFFFF_0000;
    pre.fpscr = 0x0000_00F8;
    pre.vscr = 0x0001_0000;
    let mut post = pre.clone();
    post.gpr[4] = 0xCAFE_BABE;
    post.fpscr = 0x9000_00F8;
    post.vscr = 0x0001_0001;
    CapturedRecord {
        pc: 0x0010_0000,
        raw_instruction: 0x7C00_0008, // tw
        thread_id: 0xC0E6,
        pre_state: pre,
        post_state: post,
        control_registers: ControlRegisters::Recorded,
        pre_reservation_rtime: 0x1234_5678_9ABC_DEF0,
        post_reservation_rtime: 0x1234_5678_9ABC_DEF1,
        mem_addr: 0x0000_4000,
        mem_pre: vec![0xAA, 0xBB, 0xCC, 0xDD],
        mem_post: vec![0xAA, 0xBB, 0xCC, 0xDD],
    }
}

#[test]
fn round_trip_one_record() {
    let original = sample_record();
    let bytes = synthesize_trace(std::slice::from_ref(&original));
    let parsed = read_trace_bytes(&bytes).expect("trace must parse");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0], original);
}

#[test]
fn round_trip_multiple_records() {
    let originals = [sample_record(), sample_record(), sample_record()];
    let bytes = synthesize_trace(&originals);
    let parsed = read_trace_bytes(&bytes).expect("trace must parse");
    assert_eq!(parsed.len(), 3);
    assert_eq!(parsed, originals.to_vec());
}

#[test]
fn empty_trace_with_just_header_returns_zero_records() {
    let mut buf = Vec::new();
    push_u32(&mut buf, HEADER_MAGIC);
    push_u32(&mut buf, FORMAT_VERSION);
    let parsed = read_trace_bytes(&buf).expect("header-only trace is valid");
    assert!(parsed.is_empty());
}

#[test]
fn header_magic_mismatch_is_reported() {
    let mut buf = Vec::new();
    push_u32(&mut buf, 0xDEAD_BEEFu32);
    push_u32(&mut buf, FORMAT_VERSION);
    match read_trace_bytes(&buf).unwrap_err() {
        Rpcs3CaptureError::HeaderMagic { got, expected } => {
            assert_eq!(got, 0xDEAD_BEEF);
            assert_eq!(expected, HEADER_MAGIC);
        }
        other => panic!("expected HeaderMagic, got {other:?}"),
    }
}

#[test]
fn version_mismatch_is_reported() {
    let mut buf = Vec::new();
    push_u32(&mut buf, HEADER_MAGIC);
    push_u32(&mut buf, FORMAT_VERSION + 99);
    match read_trace_bytes(&buf).unwrap_err() {
        Rpcs3CaptureError::Version { got, expected } => {
            assert_eq!(got, FORMAT_VERSION + 99);
            assert_eq!(expected, FORMAT_VERSION);
        }
        other => panic!("expected Version, got {other:?}"),
    }
}

#[test]
fn truncated_record_payload_is_reported() {
    let original = sample_record();
    let mut bytes = synthesize_trace(std::slice::from_ref(&original));
    // Drop the trailing memory bytes.
    bytes.truncate(bytes.len() - 8);
    let err = read_trace_bytes(&bytes).unwrap_err();
    matches!(err, Rpcs3CaptureError::UnexpectedEof { .. });
}

#[test]
fn corrupted_record_magic_is_reported() {
    let original = sample_record();
    let mut bytes = synthesize_trace(std::slice::from_ref(&original));
    // Header takes 8 bytes; first record-magic starts at offset 8.
    bytes[8] = 0xFF;
    bytes[9] = 0xFF;
    bytes[10] = 0xFF;
    bytes[11] = 0xFF;
    match read_trace_bytes(&bytes).unwrap_err() {
        Rpcs3CaptureError::RecordMagic { offset, got } => {
            assert_eq!(offset, 8);
            assert_eq!(got, 0xFFFF_FFFF);
        }
        other => panic!("expected RecordMagic, got {other:?}"),
    }
}

#[test]
fn a_version_4_record_carries_vrsave_fpscr_and_vscr() {
    let bytes = synthesize_trace(&[sample_record()]);
    let parsed = read_trace_bytes(&bytes).expect("trace must parse");
    let r = &parsed[0];
    assert_eq!(r.control_registers, ControlRegisters::Recorded);
    assert_eq!(
        (r.pre_state.vrsave, r.pre_state.fpscr, r.pre_state.vscr),
        (0xFFFF_0000, 0x0000_00F8, 0x0001_0000)
    );
    assert_eq!(
        (r.post_state.vrsave, r.post_state.fpscr, r.post_state.vscr),
        (0xFFFF_0000, 0x9000_00F8, 0x0001_0001)
    );
}

#[test]
fn a_version_3_record_loads_with_the_control_registers_not_recorded() {
    let mut old = sample_record();
    old.control_registers = ControlRegisters::NotRecorded;
    let bytes = synthesize_trace(std::slice::from_ref(&old));
    let parsed = read_trace_bytes(&bytes).expect("a version 3 trace still loads");
    let r = &parsed[0];
    assert_eq!(r.control_registers, ControlRegisters::NotRecorded);
    assert_eq!(r.pre_state.gpr[3], 0xDEAD_BEEF);
    assert_eq!(r.post_state.gpr[4], 0xCAFE_BABE);
    let entry = PpuStateSnapshot::capture(&PpuState::new());
    for state in [&r.pre_state, &r.post_state] {
        assert_eq!(
            (state.vrsave, state.fpscr, state.vscr),
            (entry.vrsave, entry.fpscr, entry.vscr)
        );
    }
    let case = r.to_instruction_case("v3", "v3_capture");
    assert_eq!(
        case.source.control_registers(),
        ControlRegisters::NotRecorded
    );
}

/// A record of `ori r0, r0, 0` whose post-state differs from the thread-entry pre-state only in the FPSCR.
fn nop_record(post_fpscr: u32, control: ControlRegisters) -> CapturedRecord {
    let pre = PpuStateSnapshot::capture(&PpuState::new());
    let mut post = pre.clone();
    post.fpscr = post_fpscr;
    CapturedRecord {
        pc: 0,
        raw_instruction: 0x6000_0000,
        thread_id: 1,
        pre_state: pre,
        post_state: post,
        control_registers: control,
        pre_reservation_rtime: 0,
        post_reservation_rtime: 0,
        mem_addr: 0,
        mem_pre: Vec::new(),
        mem_post: Vec::new(),
    }
}

#[test]
fn a_captured_fpscr_that_differs_is_reported_as_the_fpscr_alone() {
    // FL, a bit of the FPCC field the runner compares for an RPCS3 capture.
    let case = nop_record(0x0000_8000, ControlRegisters::Recorded).to_instruction_case("nop", "c");
    match super::super::run_case(&case) {
        super::super::CaseOutcome::StateMismatch(diff) => {
            assert_eq!(diff.fpscr, Some((0x0000_8000, 0)));
            let mut rest = diff.clone();
            rest.fpscr = None;
            assert!(rest.is_empty(), "{rest:?}");
        }
        other => panic!("expected a state mismatch, got {other:?}"),
    }
}

#[test]
fn unrecorded_vrsave_fpscr_and_vscr_are_not_compared() {
    let mut record = nop_record(0x8000_0000, ControlRegisters::NotRecorded);
    record.post_state.vrsave = !record.pre_state.vrsave;
    record.post_state.vscr = !record.pre_state.vscr;
    let case = record.to_instruction_case("nop", "c");
    assert!(matches!(
        super::super::run_case(&case),
        super::super::CaseOutcome::Pass
    ));
}

#[test]
fn reservation_rtime_round_trips_independently_of_state() {
    let mut r = sample_record();
    r.pre_reservation_rtime = 0xAAAA_BBBB_CCCC_DDDD;
    r.post_reservation_rtime = 0x1111_2222_3333_4444;
    let bytes = synthesize_trace(std::slice::from_ref(&r));
    let parsed = read_trace_bytes(&bytes).expect("trace must parse");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].pre_reservation_rtime, 0xAAAA_BBBB_CCCC_DDDD);
    assert_eq!(parsed[0].post_reservation_rtime, 0x1111_2222_3333_4444);
}

#[test]
fn captured_record_converts_into_instruction_case() {
    let r = sample_record();
    let case = r.to_instruction_case("tw_from_capture", "wipeout_run_2026_06_02");
    assert_eq!(case.label, "tw_from_capture");
    assert_eq!(case.raw_instruction, 0x7C00_0008);
    match case.source {
        OracleSource::Rpcs3Capture {
            capture_id,
            control_registers,
        } => {
            assert_eq!(capture_id, "wipeout_run_2026_06_02");
            assert_eq!(control_registers, ControlRegisters::Recorded);
        }
        other => panic!("expected Rpcs3Capture, got {other:?}"),
    }
    assert_eq!(case.initial_state.gpr[3], 0xDEAD_BEEF);
    assert_eq!(case.expected_state.gpr[4], 0xCAFE_BABE);
    assert_eq!(case.initial_memory.base, 0x0000_4000);
    assert_eq!(case.initial_memory.bytes, vec![0xAA, 0xBB, 0xCC, 0xDD]);
}
