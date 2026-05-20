use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors as errno;

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::FakeRuntime;
use crate::host::Lv2Host;
use crate::request::Lv2Request;

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
    // TB granularity is ~12.5 ns; require agreement under 1 us.
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
        Lv2Dispatch::Immediate {
            code: cellgov_time::CELL_PPU_TIMEBASE_HZ,
            effects: vec![],
        }
    );
    assert_eq!(cellgov_time::CELL_PPU_TIMEBASE_HZ, 79_800_000);
}

#[test]
fn cell_ps3_user_memory_total_is_213_mib() {
    // 213 MiB == 0x0D50_0000 == 223,346,688 bytes (PS3 game-mode
    // user-memory cap).
    assert_eq!(cellgov_ps3_abi::sys_memory::USER_MEMORY_TOTAL, 0x0D50_0000);
    assert_eq!(cellgov_ps3_abi::sys_memory::USER_MEMORY_TOTAL, 223_346_688);
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
fn stub_dispatch_returns_cell_ok_for_process_exit() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let req = Lv2Request::ProcessExit { code: 0 };
    let result = host.dispatch(req, UnitId::new(0), &rt);
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    );
}

#[test]
fn stub_dispatch_returns_cell_ok_for_unsupported() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let req = Lv2Request::Unsupported {
        number: 999,
        args: [0; 8],
    };
    let result = host.dispatch(req, UnitId::new(0), &rt);
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    );
}

#[test]
fn syscall_621_returns_ok() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 621,
            args: [0xa, 0, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    );
}

#[test]
fn syscall_512_returns_zero_non_root() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 512,
            args: [0x1000500, 0, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    );
}

#[test]
fn syscall_677_returns_ok_no_effects() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 677,
            args: [0x202, 1, 1, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    );
}

#[test]
fn syscall_362_writes_fresh_mem_id_to_args4_ptr() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 362,
            args: [
                0xFFFF_0000_0000_0000,
                0xa00000,
                0x4000_0008,
                0x400,
                0x9000,
                0,
                0,
                0,
            ],
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert_eq!(effects.len(), 1);
            if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                assert_eq!(range.start().raw(), 0x9000);
                assert_eq!(range.length(), 4);
                let id = u32::from_be_bytes(bytes.bytes().try_into().unwrap());
                assert_ne!(id, 0);
            } else {
                panic!("expected SharedWriteIntent");
            }
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn syscall_324_writes_fresh_cid_to_out_ptr() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 324,
            args: [0x9000, 0xa00000, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert_eq!(effects.len(), 1);
            if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                assert_eq!(range.start().raw(), 0x9000);
                assert_eq!(range.length(), 4);
                let cid = u32::from_be_bytes(bytes.bytes().try_into().unwrap());
                assert_ne!(cid, 0, "kernel id must be nonzero");
            } else {
                panic!("expected SharedWriteIntent");
            }
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn syscall_334_returns_ok_no_effects() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 334,
            args: [0x3000_0000, 0x4000_0007, 0x40000, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    );
}

#[test]
fn syscall_337_writes_start_addr_to_alloc_addr_ptr() {
    // sys_mmapper_search_and_map: a start_addr inside the valid
    // VM range yields CELL_OK and writes start_addr verbatim to
    // *alloc_addr_ptr (flat backing: the search collapses to
    // "place the shm at start_addr").
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 337,
            args: [0x3000_0000, 0x4000_0008, 0x200, 0x9000, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert_eq!(effects.len(), 1);
            if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                assert_eq!(range.start().raw(), 0x9000);
                assert_eq!(range.length(), 4);
                assert_eq!(bytes.bytes(), &0x3000_0000u32.to_be_bytes());
            } else {
                panic!("expected SharedWriteIntent");
            }
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn syscall_337_rejects_out_of_range_start_addr() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let below = host.dispatch(
        Lv2Request::Unsupported {
            number: 337,
            args: [0x1000_0000, 0x4000_0008, 0x200, 0x9000, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        below,
        Lv2Dispatch::Immediate {
            code: errno::CELL_EINVAL.into(),
            effects: vec![],
        }
    );
    let above = host.dispatch(
        Lv2Request::Unsupported {
            number: 337,
            args: [0xC000_0000, 0x4000_0008, 0x200, 0x9000, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        above,
        Lv2Dispatch::Immediate {
            code: errno::CELL_EINVAL.into(),
            effects: vec![],
        }
    );
}

#[test]
fn syscall_330_writes_monotonic_256mib_aligned_address() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let first = host.dispatch(
        Lv2Request::Unsupported {
            number: 330,
            args: [0x1000_0000, 0x400, 0, 0x9000, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    let first_addr = match first {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert_eq!(effects.len(), 1);
            if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                assert_eq!(range.start().raw(), 0x9000);
                assert_eq!(range.length(), 4);
                u32::from_be_bytes(bytes.bytes().try_into().unwrap())
            } else {
                panic!("expected SharedWriteIntent");
            }
        }
        other => panic!("expected Immediate, got {other:?}"),
    };
    assert_eq!(
        first_addr & 0x0FFF_FFFF,
        0,
        "first addr must be 256 MiB aligned"
    );
    let second = host.dispatch(
        Lv2Request::Unsupported {
            number: 330,
            args: [0x1000_0000, 0x400, 0, 0x9100, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    let second_addr = match second {
        Lv2Dispatch::Immediate { effects, .. } => {
            if let Effect::SharedWriteIntent { bytes, .. } = &effects[0] {
                u32::from_be_bytes(bytes.bytes().try_into().unwrap())
            } else {
                panic!("expected SharedWriteIntent");
            }
        }
        other => panic!("expected Immediate, got {other:?}"),
    };
    assert_eq!(second_addr, first_addr + 0x1000_0000);
}

#[test]
fn syscall_330_returns_enomem_when_cursor_would_cross_kernel_stack_region() {
    // mmapper_alloc bumps a 256 MiB-aligned cursor starting at
    // SYS_RSX_MEM_END (0x4000_0000) and refuses grants that would
    // cross MMAPPER_REGION_END (0xD000_0000) into the kernel-reserved
    // PPU stack region. The window holds nine 256 MiB grants; the
    // tenth must surface CELL_ENOMEM.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let req = || Lv2Request::Unsupported {
        number: 330,
        args: [0x1000_0000, 0, 0, 0x9000, 0, 0, 0, 0],
    };
    for i in 0..9 {
        let result = host.dispatch(req(), UnitId::new(0), &rt);
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0, "expected OK on call {}", i + 1);
                assert_eq!(effects.len(), 1);
            }
            other => panic!("call {}: expected Immediate, got {other:?}", i + 1),
        }
    }
    let exhausted = host.dispatch(req(), UnitId::new(0), &rt);
    assert_eq!(
        exhausted,
        Lv2Dispatch::Immediate {
            code: errno::CELL_ENOMEM.into(),
            effects: vec![],
        },
        "the 10th 256 MiB allocation must cap-fail and surface CELL_ENOMEM"
    );
}

#[test]
fn syscall_494_flags_without_bit2_returns_ok_no_effects() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 494,
            args: [0, 0x9000, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    );
}

#[test]
fn syscall_494_flags_with_bit2_writes_zero_count_at_offset_0x10() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 494,
            args: [0x2, 0x9000, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert_eq!(effects.len(), 1);
            if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                assert_eq!(range.start().raw(), 0x9010);
                assert_eq!(range.length(), 4);
                assert_eq!(bytes.bytes(), &0u32.to_be_bytes());
            } else {
                panic!("expected SharedWriteIntent");
            }
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn syscall_136_returns_ok() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 136,
            args: [0x4000_0002, 0x4000_0001, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    );
}

#[test]
fn syscall_48_writes_priority_to_priop() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 48,
            args: [0x0100_0000, 0x9000, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert_eq!(effects.len(), 1);
            if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                assert_eq!(range.start().raw(), 0x9000);
                assert_eq!(range.length(), 4);
                assert_eq!(bytes.bytes(), &1001u32.to_be_bytes());
            } else {
                panic!("expected SharedWriteIntent");
            }
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn syscall_332_writes_fresh_mem_id_to_mem_id_ptr() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let first_call = host.dispatch(
        Lv2Request::Unsupported {
            number: 332,
            args: [0x8006_0100_0000_0010, 0x10000, 0x200, 0x9000, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    let first_id = match first_call {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert_eq!(effects.len(), 1);
            if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                assert_eq!(range.start().raw(), 0x9000);
                assert_eq!(range.length(), 4);
                u32::from_be_bytes(bytes.bytes().try_into().unwrap())
            } else {
                panic!("expected SharedWriteIntent");
            }
        }
        other => panic!("expected Immediate, got {other:?}"),
    };
    let second_call = host.dispatch(
        Lv2Request::Unsupported {
            number: 332,
            args: [0x8006_0100_0000_0010, 0x10000, 0x200, 0x9100, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    let second_id = match second_call {
        Lv2Dispatch::Immediate { effects, .. } => {
            if let Effect::SharedWriteIntent { bytes, .. } = &effects[0] {
                u32::from_be_bytes(bytes.bytes().try_into().unwrap())
            } else {
                panic!("expected SharedWriteIntent");
            }
        }
        other => panic!("expected Immediate, got {other:?}"),
    };
    assert!(
        second_id > first_id,
        "successive mem_ids must be monotonic: first=0x{first_id:x} second=0x{second_id:x}"
    );
}

#[test]
fn syscall_480_returns_registered_kernel_id_for_known_stem() {
    // _sys_prx_load_module with a path whose stem matches a
    // registered PRX returns the registry's kernel id, not the
    // path-pointer fallback.
    let mut host = Lv2Host::new();
    let expected_id = host.prx_registry_mut().register(
        "libaudio".into(),
        "cellAudio_Library".into(),
        0x0147_0000,
        0x0148_0000,
        0x0147_da30,
        None,
        None,
    );
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    let path = b"/dev_flash/sys/external/libaudio.sprx\0";
    let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0x4000), path.len() as u64)
        .unwrap();
    mem.apply_commit(range, path).unwrap();
    let rt = FakeRuntime::with_memory(mem);

    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 480,
            args: [0x4000, 0, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: u64::from(expected_id),
            effects: vec![],
        }
    );
}

#[test]
fn syscall_480_unknown_path_falls_back_to_pointer_echo() {
    let mut host = Lv2Host::new();
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    let path = b"external/libnotfound.sprx\0";
    mem.apply_commit(
        cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0x5000), path.len() as u64)
            .unwrap(),
        path,
    )
    .unwrap();
    let rt = FakeRuntime::with_memory(mem);

    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 480,
            args: [0x5000, 0, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: 0x5000,
            effects: vec![],
        }
    );
}

#[test]
fn syscall_497_routes_through_same_resolver_as_480() {
    let mut host = Lv2Host::new();
    let expected_id = host.prx_registry_mut().register(
        "libaudio".into(),
        "cellAudio_Library".into(),
        0x0147_0000,
        0x0148_0000,
        0x0147_da30,
        None,
        None,
    );
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    let path = b"external/libaudio.sprx\0";
    mem.apply_commit(
        cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0x4000), path.len() as u64)
            .unwrap(),
        path,
    )
    .unwrap();
    let rt = FakeRuntime::with_memory(mem);

    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 497,
            args: [0x4000, 0xCAFEBABE, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: u64::from(expected_id),
            effects: vec![],
        }
    );
}

#[test]
fn syscall_494_walks_registry_writing_ids_and_count() {
    let mut host = Lv2Host::new();
    let liblv2_id = host.prx_registry_mut().register(
        "liblv2".into(),
        "liblv2".into(),
        0x0145_0000,
        0x0146_0000,
        0x0145_d000,
        None,
        None,
    );
    let audio_id = host.prx_registry_mut().register(
        "libaudio".into(),
        "cellAudio_Library".into(),
        0x0147_0000,
        0x0148_0000,
        0x0147_da30,
        None,
        None,
    );
    // pInfo struct at 0x4000:
    //   size@0 = 0x20, pad@8 = 0, max@0xC = 8,
    //   count@0x10 (out), idlist@0x14 = 0x4040, unk@0x18 = 0
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    let mut p_info = [0u8; 0x20];
    p_info[0..8].copy_from_slice(&0x20u64.to_be_bytes());
    p_info[0x0C..0x10].copy_from_slice(&8u32.to_be_bytes());
    p_info[0x14..0x18].copy_from_slice(&0x4040u32.to_be_bytes());
    mem.apply_commit(
        cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0x4000), p_info.len() as u64)
            .unwrap(),
        &p_info,
    )
    .unwrap();
    let rt = FakeRuntime::with_memory(mem);

    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 494,
            args: [0x2, 0x4000, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            // One entry (audio; liblv2 filtered) + count write = 2 effects.
            assert_eq!(effects.len(), 2);
            if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                assert_eq!(range.start().raw(), 0x4040);
                assert_eq!(
                    u32::from_be_bytes(bytes.bytes().try_into().unwrap()),
                    audio_id
                );
            }
            if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[1] {
                assert_eq!(range.start().raw(), 0x4010);
                assert_eq!(u32::from_be_bytes(bytes.bytes().try_into().unwrap()), 1);
            }
            assert!(liblv2_id > 0);
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn syscall_486_returns_ok() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 486,
            args: [0; 8],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    );
}

#[test]
fn syscall_484_returns_elf_is_registered() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 484,
            args: [0; 8],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: 0x8001_1910,
            effects: vec![],
        }
    );
}

#[test]
fn syscall_462_returns_enosys() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 462,
            args: [0; 8],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: errno::CELL_ENOSYS.into(),
            effects: vec![],
        }
    );
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
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: errno::CELL_EIO.into(),
            effects: vec![],
        }
    );
}

#[test]
fn prx_start_module_writes_no_start_sentinel_to_p_opt_entry() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let p_opt: u64 = 0x4000_1000;
    let mut args = [0u64; 8];
    args[0] = 0x1234;
    args[2] = p_opt;
    let result = host.dispatch(
        Lv2Request::Unsupported { number: 481, args },
        UnitId::new(0),
        &rt,
    );
    let effects = match result {
        Lv2Dispatch::Immediate { code: 0, effects } => effects,
        other => panic!("expected Immediate{{code:0}}, got {other:?}"),
    };
    assert_eq!(effects.len(), 1, "expected exactly one write effect");
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), (p_opt + 16));
            assert_eq!(range.length(), 8);
            assert_eq!(bytes.bytes(), &u64::MAX.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn prx_load_module_returns_r3_as_synthetic_id() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let path_ptr: u64 = 0x0146_2d58;
    let mut args = [0u64; 8];
    args[0] = path_ptr;
    let result = host.dispatch(
        Lv2Request::Unsupported { number: 480, args },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: path_ptr,
            effects: vec![],
        },
        "syscall 480 must echo r3 as the synthesised module ID"
    );
}

#[test]
fn syscall_481_rejects_zero_id_with_einval() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let mut args = [0u64; 8];
    args[0] = 0;
    args[2] = 0x4000_1000;
    let result = host.dispatch(
        Lv2Request::Unsupported { number: 481, args },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: errno::CELL_EINVAL.into(),
            effects: vec![],
        }
    );
}

#[test]
fn syscall_481_rejects_zero_p_opt_with_einval() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let mut args = [0u64; 8];
    args[0] = 0x1234;
    args[2] = 0;
    let result = host.dispatch(
        Lv2Request::Unsupported { number: 481, args },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: errno::CELL_EINVAL.into(),
            effects: vec![],
        }
    );
}

#[test]
fn syscall_494_rejects_null_p_info_with_efault() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 494,
            args: [0x2, 0, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: errno::CELL_EFAULT.into(),
            effects: vec![],
        }
    );
}

#[test]
fn syscall_494_idlist_order_is_independent_of_registration_order() {
    // Registry uses BTreeMap-keyed kernel ids; ordering must
    // depend on allocation order, not insertion order. Two hosts
    // register A-then-B vs B-then-A: assert idlist bytes match.
    fn idlist_bytes(register: impl FnOnce(&mut Lv2Host)) -> Vec<u8> {
        let mut host = Lv2Host::new();
        register(&mut host);
        let mut mem = cellgov_mem::GuestMemory::new(0x10000);
        let mut p_info = [0u8; 0x20];
        p_info[0..8].copy_from_slice(&0x20u64.to_be_bytes());
        p_info[0x0C..0x10].copy_from_slice(&8u32.to_be_bytes());
        p_info[0x14..0x18].copy_from_slice(&0x4040u32.to_be_bytes());
        mem.apply_commit(
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0x4000), p_info.len() as u64)
                .unwrap(),
            &p_info,
        )
        .unwrap();
        let rt = FakeRuntime::with_memory(mem);
        let result = host.dispatch(
            Lv2Request::Unsupported {
                number: 494,
                args: [0x2, 0x4000, 0, 0, 0, 0, 0, 0],
            },
            UnitId::new(0),
            &rt,
        );
        let effects = match result {
            Lv2Dispatch::Immediate { effects, .. } => effects,
            other => panic!("expected Immediate, got {other:?}"),
        };
        let mut all = Vec::new();
        for eff in &effects {
            if let Effect::SharedWriteIntent { bytes, .. } = eff {
                all.extend_from_slice(bytes.bytes());
            }
        }
        all
    }
    let a_first = idlist_bytes(|h| {
        h.prx_registry_mut().register(
            "libaudio".into(),
            "cellAudio_Library".into(),
            0x0147_0000,
            0x0148_0000,
            0x0147_da30,
            None,
            None,
        );
        h.prx_registry_mut().register(
            "libfiber".into(),
            "cellFiber_Library".into(),
            0x0149_0000,
            0x014a_0000,
            0x0149_da30,
            None,
            None,
        );
    });
    let b_first = idlist_bytes(|h| {
        h.prx_registry_mut().register(
            "libfiber".into(),
            "cellFiber_Library".into(),
            0x0149_0000,
            0x014a_0000,
            0x0149_da30,
            None,
            None,
        );
        h.prx_registry_mut().register(
            "libaudio".into(),
            "cellAudio_Library".into(),
            0x0147_0000,
            0x0148_0000,
            0x0147_da30,
            None,
            None,
        );
    });
    assert_eq!(
        a_first, b_first,
        "syscall 494 idlist bytes diverged across registration orders -- \
         prx_registry iteration order is leaking into guest memory"
    );
}

#[test]
fn ss_access_control_engine_pkg_id_1_returns_enosys() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::SsAccessControlEngine {
            pkg_id: 1,
            a2: 0x9000,
            a3: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: errno::CELL_ENOSYS.into(),
            effects: vec![],
        }
    );
}

#[test]
fn ss_access_control_engine_pkg_id_2_writes_program_authority_id() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::SsAccessControlEngine {
            pkg_id: 2,
            a2: 0x9000,
            a3: 0,
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert_eq!(effects.len(), 1);
            if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                assert_eq!(range.start().raw(), 0x9000);
                assert_eq!(range.length(), 8);
                let v = u64::from_be_bytes(bytes.bytes().try_into().unwrap());
                assert_eq!(v, 0x1070_0000_3A00_0001);
            } else {
                panic!("expected SharedWriteIntent");
            }
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn ss_access_control_engine_pkg_id_2_efault_on_zero_a2() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::SsAccessControlEngine {
            pkg_id: 2,
            a2: 0,
            a3: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: errno::CELL_EFAULT.into(),
            effects: vec![],
        }
    );
}

#[test]
fn ss_access_control_engine_pkg_id_2_efault_when_a2_overflows_u32() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::SsAccessControlEngine {
            pkg_id: 2,
            a2: u64::from(u32::MAX) + 1,
            a3: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: errno::CELL_EFAULT.into(),
            effects: vec![],
        }
    );
}

#[test]
fn ss_access_control_engine_default_pkg_id_returns_ss_status() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::SsAccessControlEngine {
            pkg_id: 99,
            a2: 0,
            a3: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: 0x8001_051D,
            effects: vec![],
        }
    );
}

#[test]
fn memory_get_user_memory_size_writes_total_then_available() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::MemoryGetUserMemorySize {
            mem_info_ptr: 0x9000,
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert_eq!(effects.len(), 1);
            if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                assert_eq!(range.start().raw(), 0x9000);
                assert_eq!(range.length(), 8);
                let total = u32::from_be_bytes(bytes.bytes()[0..4].try_into().unwrap());
                let avail = u32::from_be_bytes(bytes.bytes()[4..8].try_into().unwrap());
                assert_eq!(total, 0x0D50_0000);
                assert_eq!(avail, 0x0D50_0000);
            } else {
                panic!("expected SharedWriteIntent");
            }
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn memory_get_user_memory_size_efault_on_null_ptr() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::MemoryGetUserMemorySize { mem_info_ptr: 0 },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: errno::CELL_EFAULT.into(),
            effects: vec![],
        }
    );
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
        Lv2Dispatch::Immediate {
            code: errno::CELL_EFAULT.into(),
            effects: vec![],
        }
    );
}

#[test]
fn immediate_write_u32_efault_on_null_ptr() {
    let host = Lv2Host::new();
    let result = host.immediate_write_u32(0xCAFE, 0, UnitId::new(0));
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: errno::CELL_EFAULT.into(),
            effects: vec![],
        }
    );
}

#[test]
fn memory_free_is_no_op_returning_ok() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(Lv2Request::MemoryFree { addr: 0x1000 }, UnitId::new(0), &rt);
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    );
}

#[test]
fn ppu_thread_yield_is_no_op_returning_ok() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(Lv2Request::PpuThreadYield, UnitId::new(0), &rt);
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    );
}

#[test]
fn malformed_request_records_invariant_break_and_returns_einval() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let before = host.invariant_break_count();
    let result = host.dispatch(
        Lv2Request::Malformed {
            number: 99,
            reason: "test",
            args: [0; 8],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: errno::CELL_EINVAL.into(),
            effects: vec![],
        }
    );
    assert!(host.invariant_break_count() > before);
}

#[test]
fn hypercall_records_invariant_break_and_returns_einval() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let before = host.invariant_break_count();
    let result = host.dispatch(
        Lv2Request::Hypercall {
            lev: std::num::NonZeroU8::new(1).unwrap(),
            r11: 0xCAFE,
            args: [0; 8],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: errno::CELL_EINVAL.into(),
            effects: vec![],
        }
    );
    assert!(host.invariant_break_count() > before);
}

#[test]
fn spu_thread_group_terminate_logs_invariant_break() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let before = host.invariant_break_count();
    let result = host.dispatch(
        Lv2Request::SpuThreadGroupTerminate {
            group_id: 1,
            value: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    );
    assert!(host.invariant_break_count() > before);
}

#[test]
fn ppu_thread_create_logs_invariant_break_on_nonzero_flags() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let before = host.invariant_break_count();
    let _ = host.dispatch(
        Lv2Request::PpuThreadCreate {
            id_ptr: 0x9000,
            entry_opd: 0x4000_0000,
            arg: 0,
            priority: 1000,
            stacksize: 0x4000,
            flags: 0x1, // JOINABLE -- unmodeled, must surface
        },
        UnitId::new(0),
        &rt,
    );
    assert!(
        host.invariant_break_count() > before,
        "expected log_invariant_break to fire on nonzero flags"
    );
}
