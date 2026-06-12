//! TTY read/write and time/timezone syscall dispatch, plus out-pointer EFAULT gates.

use super::*;

#[test]
fn tty_write_writes_nwritten_and_returns_ok() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let req = Lv2Request::TtyWrite {
        fd: 0,
        buf_ptr: 0x8000,
        len: 64,
        nwritten_ptr: 0x9000,
    };
    let result = host.dispatch(req, UnitId::new(0), &rt);
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert_eq!(effects.len(), 1);
            if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                assert_eq!(range.start().raw(), 0x9000);
                assert_eq!(range.length(), 4);
                assert_eq!(bytes.bytes(), &64u32.to_be_bytes());
            } else {
                panic!("expected SharedWriteIntent");
            }
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn tty_write_appends_buffer_bytes_to_tty_log() {
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    mem.apply_commit(ByteRange::contiguous_u32(0x8000, 12), b"hello world\n")
        .unwrap();
    let rt = FakeRuntime::with_memory(mem);
    let mut host = Lv2Host::new();
    host.dispatch(
        Lv2Request::TtyWrite {
            fd: 1,
            buf_ptr: 0x8000,
            len: 12,
            nwritten_ptr: 0x9000,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(host.tty_log(), b"hello world\n");
}

#[test]
fn tty_write_concatenates_across_calls_in_dispatch_order() {
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    mem.apply_commit(ByteRange::contiguous_u32(0x8000, 4), b"abcd")
        .unwrap();
    mem.apply_commit(ByteRange::contiguous_u32(0x8100, 3), b"xyz")
        .unwrap();
    let rt = FakeRuntime::with_memory(mem);
    let mut host = Lv2Host::new();
    host.dispatch(
        Lv2Request::TtyWrite {
            fd: 1,
            buf_ptr: 0x8000,
            len: 4,
            nwritten_ptr: 0x9000,
        },
        UnitId::new(0),
        &rt,
    );
    host.dispatch(
        Lv2Request::TtyWrite {
            fd: 1,
            buf_ptr: 0x8100,
            len: 3,
            nwritten_ptr: 0x9000,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(host.tty_log(), b"abcdxyz");
}

#[test]
fn tty_write_zero_len_is_a_noop_for_tty_log() {
    let rt = FakeRuntime::new(0x10000);
    let mut host = Lv2Host::new();
    host.dispatch(
        Lv2Request::TtyWrite {
            fd: 1,
            buf_ptr: 0x8000,
            len: 0,
            nwritten_ptr: 0x9000,
        },
        UnitId::new(0),
        &rt,
    );
    assert!(host.tty_log().is_empty());
}

#[test]
fn tty_write_unmapped_buf_does_not_corrupt_tty_log_and_still_returns_ok() {
    let rt = FakeRuntime::new(0x1000);
    let mut host = Lv2Host::new();
    let result = host.dispatch(
        Lv2Request::TtyWrite {
            fd: 1,
            buf_ptr: 0x8000,
            len: 4,
            nwritten_ptr: 0x100,
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, .. } => assert_eq!(code, 0),
        other => panic!("expected Immediate, got {other:?}"),
    }
    assert!(host.tty_log().is_empty());
}

#[test]
fn time_get_current_time_writes_sec_and_nsec_at_zero_tick() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::TimeGetCurrentTime {
            sec_ptr: 0x8000,
            nsec_ptr: 0x8008,
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert_eq!(effects.len(), 2);
            for eff in &effects {
                if let Effect::SharedWriteIntent { range, bytes, .. } = eff {
                    assert_eq!(range.length(), 8);
                    assert_eq!(bytes.bytes(), &0u64.to_be_bytes());
                } else {
                    panic!("expected SharedWriteIntent");
                }
            }
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn time_get_current_time_splits_at_billion_tick() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000).with_tick(cellgov_time::GuestTicks::new(1_500_000_001));
    let result = host.dispatch(
        Lv2Request::TimeGetCurrentTime {
            sec_ptr: 0x1000,
            nsec_ptr: 0x1008,
        },
        UnitId::new(0),
        &rt,
    );
    let effects = match result {
        Lv2Dispatch::Immediate { effects, .. } => effects,
        other => panic!("expected Immediate, got {other:?}"),
    };
    if let Effect::SharedWriteIntent { bytes, .. } = &effects[0] {
        let v = u64::from_be_bytes(bytes.bytes().try_into().unwrap());
        assert_eq!(v, 1);
    } else {
        panic!();
    }
    if let Effect::SharedWriteIntent { bytes, .. } = &effects[1] {
        let v = u64::from_be_bytes(bytes.bytes().try_into().unwrap());
        assert_eq!(v, 500_000_001);
    } else {
        panic!();
    }
}

#[test]
fn time_get_current_time_and_timebase_frequency_are_coherent() {
    let tick_later = 3 * cellgov_time::SIMULATED_INSTRUCTIONS_PER_SECOND + 500_000_000;
    let (sec, nsec) = cellgov_time::ticks_to_sec_nsec(tick_later);
    let tb = cellgov_time::ticks_to_tb(tick_later);
    let as_nsec_from_time_syscall = sec * 1_000_000_000 + nsec;
    let us_from_tb = tb * 1_000_000 / cellgov_time::CELL_PPU_TIMEBASE_HZ;
    let nsec_from_tb = us_from_tb * 1_000;
    let diff = as_nsec_from_time_syscall.abs_diff(nsec_from_tb);
    assert!(
        diff < 1_000,
        "time syscall and mftb must agree within 1 us: got {diff} ns"
    );
}

#[test]
fn time_get_timebase_frequency_returns_cell_ppu_timebase_hz() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let result = host.dispatch(Lv2Request::TimeGetTimebaseFrequency, UnitId::new(0), &rt);
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cellgov_time::CELL_PPU_TIMEBASE_HZ)
    );
    assert_eq!(cellgov_time::CELL_PPU_TIMEBASE_HZ, 79_800_000);
}

#[test]
fn time_get_timezone_writes_zero_through_both_out_pointers() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let result = host.dispatch(
        Lv2Request::TimeGetTimezone {
            timezone_ptr: 0xd000_fd10,
            summer_time_ptr: 0xd000_fd14,
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert_eq!(effects.len(), 2);
            let expected_zero = 0i32.to_be_bytes();
            if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                assert_eq!(range.start().raw(), 0xd000_fd10);
                assert_eq!(range.length(), 4);
                assert_eq!(bytes.bytes(), &expected_zero);
            } else {
                panic!("expected SharedWriteIntent for timezone_ptr");
            }
            if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[1] {
                assert_eq!(range.start().raw(), 0xd000_fd14);
                assert_eq!(range.length(), 4);
                assert_eq!(bytes.bytes(), &expected_zero);
            } else {
                panic!("expected SharedWriteIntent for summer_time_ptr");
            }
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn tty_read_returns_eio() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 402,
            args: [0; 8],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(result, Lv2Dispatch::immediate(cell_errors::CELL_EIO.into()));
}

#[test]
fn time_get_timezone_efault_on_null_ptr() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::TimeGetTimezone {
            timezone_ptr: 0,
            summer_time_ptr: 0x9004,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
}

#[test]
fn immediate_write_u32_efault_on_null_ptr() {
    let host = Lv2Host::new();
    let result = host.immediate_write_u32(0xCAFE, 0, UnitId::new(0));
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
}
