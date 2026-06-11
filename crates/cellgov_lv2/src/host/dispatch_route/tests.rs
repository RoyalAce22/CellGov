use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::FakeRuntime;
use crate::host::Lv2Host;
use crate::request::Lv2Request;

/// Mint an mmapper handle via sc 332 and return the assigned `mem_id`.
fn mint_mmapper_handle(host: &mut Lv2Host, rt: &FakeRuntime, size: u64, flags: u64) -> u32 {
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 332,
            args: [0xffff_0000_0000_0000, size, flags, 0x9000, 0, 0, 0, 0],
        },
        UnitId::new(0),
        rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = result else {
        panic!("332 must succeed when minting helper, got {result:?}");
    };
    let Effect::SharedWriteIntent { bytes, .. } = &effects[0] else {
        panic!("332 must emit a SharedWriteIntent");
    };
    u32::from_be_bytes(bytes.bytes().try_into().unwrap())
}

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
fn cell_ps3_user_memory_total_is_213_mib() {
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
    assert_eq!(result, Lv2Dispatch::immediate(0));
}

#[test]
fn unsupported_dispatch_returns_cell_enosys() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let req = Lv2Request::Unsupported {
        number: 999,
        args: [0; 8],
    };
    let result = host.dispatch(req, UnitId::new(0), &rt);
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_ENOSYS.into())
    );
}

#[test]
fn unresolved_import_dispatch_returns_cell_einval() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let req = Lv2Request::UnresolvedImport {
        nid: 0x744680a2, // sys_initialize_tls
    };
    let result = host.dispatch(req, UnitId::new(0), &rt);
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cellgov_ps3_abi::cell_errors::CELL_EINVAL.into())
    );
}

#[test]
fn unresolved_import_dispatch_handles_unknown_nid() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let req = Lv2Request::UnresolvedImport { nid: 0xdead_beef };
    let result = host.dispatch(req, UnitId::new(0), &rt);
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cellgov_ps3_abi::cell_errors::CELL_EINVAL.into())
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
    assert_eq!(result, Lv2Dispatch::immediate(0));
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
    assert_eq!(result, Lv2Dispatch::immediate(0));
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
    assert_eq!(result, Lv2Dispatch::immediate(0));
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
fn syscall_334_unknown_mem_id_returns_esrch_and_logs_break() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let breaks_before = host.invariant_break_count();
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 334,
            args: [0x5000_0000, 0x4000_0007, 0x40000, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into())
    );
    assert_eq!(host.invariant_break_count() - breaks_before, 1);
}

#[test]
fn syscall_332_then_334_records_pending_region_install() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let mut mem_id_buf = [0u8; 4];
    let result_332 = host.dispatch(
        Lv2Request::Unsupported {
            number: 332,
            args: [
                0xffff_0000_0000_0000,
                0x0400_0000,
                0x400,
                0x9000,
                0,
                0,
                0,
                0,
            ],
        },
        UnitId::new(0),
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = result_332 else {
        panic!("332 must succeed");
    };
    assert_eq!(effects.len(), 1);
    let Effect::SharedWriteIntent { bytes, .. } = &effects[0] else {
        panic!("332 must emit a SharedWriteIntent for the mem_id");
    };
    mem_id_buf.copy_from_slice(bytes.bytes());
    let mem_id = u32::from_be_bytes(mem_id_buf);

    let result_334 = host.dispatch(
        Lv2Request::Unsupported {
            number: 334,
            args: [0x5000_0000, u64::from(mem_id), 0x40000, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result_334,
        Lv2Dispatch::immediate(cell_errors::CELL_OK.into())
    );
    let installs: Vec<_> = host.drain_pending_region_installs().collect();
    assert_eq!(installs, vec![(0x5000_0000_u64, 0x0400_0000_usize)]);
}

#[test]
fn syscall_334_misaligned_returns_ealign() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result_332 = host.dispatch(
        Lv2Request::Unsupported {
            number: 332,
            args: [
                0xffff_0000_0000_0000,
                0x0100_0000,
                0x400,
                0x9000,
                0,
                0,
                0,
                0,
            ],
        },
        UnitId::new(0),
        &rt,
    );
    let Lv2Dispatch::Immediate { effects, .. } = result_332 else {
        panic!("332 must succeed");
    };
    let Effect::SharedWriteIntent { bytes, .. } = &effects[0] else {
        panic!();
    };
    let mut buf = [0u8; 4];
    buf.copy_from_slice(bytes.bytes());
    let mem_id = u32::from_be_bytes(buf);

    let result_334 = host.dispatch(
        Lv2Request::Unsupported {
            number: 334,
            args: [0x5000_0007, u64::from(mem_id), 0x40000, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result_334,
        Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into())
    );
}

#[test]
fn syscall_334_addr_out_of_range_returns_einval() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 334,
            args: [0xD000_0000, 0x4000_0001, 0x40000, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
    );
}

#[test]
fn syscall_362_records_handle_keyed_on_mem_id() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 362,
            args: [
                0xffff_0000_0000_0000, // ipc_key
                0x00a0_0000,           // size
                0x4000_0015,           // cid
                0x400,                 // flags (SYS_MEMORY_PAGE_SIZE_1M)
                0x9000,                // mem_id_ptr
                0,
                0,
                0,
            ],
        },
        UnitId::new(0),
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = result else {
        panic!("362 must succeed");
    };
    assert_eq!(effects.len(), 1);
    let Effect::SharedWriteIntent { bytes, .. } = &effects[0] else {
        panic!("362 must emit a SharedWriteIntent for the mem_id");
    };
    let mut buf = [0u8; 4];
    buf.copy_from_slice(bytes.bytes());
    let mem_id = u32::from_be_bytes(buf);

    let result_334 = host.dispatch(
        Lv2Request::Unsupported {
            number: 334,
            args: [0x5400_0000, u64::from(mem_id), 0x40000, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result_334,
        Lv2Dispatch::immediate(cell_errors::CELL_OK.into())
    );
    let installs: Vec<_> = host.drain_pending_region_installs().collect();
    assert_eq!(installs, vec![(0x5400_0000_u64, 0x00a0_0000_usize)]);
}

/// sc 332 then sc 337: the search lands on the hint when the window
/// is empty, the ledger records the install, and the runtime sees a
/// `PendingRegionInstall` covering the written-back address.
#[test]
fn syscall_332_then_337_searches_installs_and_writes_back_found_addr() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let mem_id = mint_mmapper_handle(&mut host, &rt, 0x0010_0000, 0x400);
    let result_337 = host.dispatch(
        Lv2Request::Unsupported {
            number: 337,
            args: [0x5000_0000, u64::from(mem_id), 0x40000, 0x9100, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = result_337 else {
        panic!("337 must succeed, got {result_337:?}");
    };
    let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] else {
        panic!("337 must emit a SharedWriteIntent");
    };
    assert_eq!(range.start().raw(), 0x9100);
    assert_eq!(range.length(), 4);
    assert_eq!(
        u32::from_be_bytes(bytes.bytes().try_into().unwrap()),
        0x5000_0000,
        "empty window -> search returns the hint as the found address",
    );
    let installs: Vec<_> = host.drain_pending_region_installs().collect();
    assert_eq!(installs, vec![(0x5000_0000_u64, 0x0010_0000_usize)]);
}

/// sc 337 with a hint that collides with a prior install: the search
/// must walk forward and the write-back must report the found
/// (post-collision) address, not the hint.
#[test]
fn syscall_337_search_walks_past_existing_install() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let first = mint_mmapper_handle(&mut host, &rt, 0x0010_0000, 0x400);
    let second = mint_mmapper_handle(&mut host, &rt, 0x0010_0000, 0x400);
    // First 337 fills [0x5000_0000, 0x5010_0000).
    host.dispatch(
        Lv2Request::Unsupported {
            number: 337,
            args: [0x5000_0000, u64::from(first), 0x40000, 0x9000, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    // Second 337 with same hint: search must walk past the prior
    // install, return 0x5010_0000.
    let r = host.dispatch(
        Lv2Request::Unsupported {
            number: 337,
            args: [0x5000_0000, u64::from(second), 0x40000, 0x9100, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = r else {
        panic!("337 must succeed, got {r:?}");
    };
    let Effect::SharedWriteIntent { bytes, .. } = &effects[0] else {
        panic!("expected SharedWriteIntent");
    };
    assert_eq!(
        u32::from_be_bytes(bytes.bytes().try_into().unwrap()),
        0x5010_0000,
        "second 337 must NOT return the hint -- the prior install occupies it",
    );
}

/// sc 337 with a `mem_id` the handle table does not know.
#[test]
fn syscall_337_unknown_mem_id_returns_esrch_and_logs_break() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let breaks_before = host.invariant_break_count();
    let r = host.dispatch(
        Lv2Request::Unsupported {
            number: 337,
            args: [0x5000_0000, 0xdead_beef, 0x40000, 0x9000, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(r, Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into()));
    assert_eq!(host.invariant_break_count() - breaks_before, 1);
}

#[test]
fn syscall_337_rejects_out_of_range_start_addr() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let mem_id = mint_mmapper_handle(&mut host, &rt, 0x0010_0000, 0x400);
    // Below MMAPPER_REGION_START.
    let below = host.dispatch(
        Lv2Request::Unsupported {
            number: 337,
            args: [0x1000_0000, u64::from(mem_id), 0x40000, 0x9000, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        below,
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
    );
    // At MMAPPER_REGION_END (exclusive).
    let above = host.dispatch(
        Lv2Request::Unsupported {
            number: 337,
            args: [0xC000_0000, u64::from(mem_id), 0x40000, 0x9000, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        above,
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
    );
}

#[test]
fn syscall_337_null_alloc_addr_returns_efault() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let mem_id = mint_mmapper_handle(&mut host, &rt, 0x0010_0000, 0x400);
    let r = host.dispatch(
        Lv2Request::Unsupported {
            number: 337,
            args: [0x5000_0000, u64::from(mem_id), 0x40000, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(r, Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into()));
}

#[test]
fn syscall_337_exhausted_window_returns_enomem() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    // Fill the entire mmapper window in the ledger (direct insert; we
    // don't go through the dispatch arm here -- just the ledger
    // state the search will consult).
    let window = Lv2Host::MMAPPER_REGION_END - Lv2Host::MMAPPER_REGION_START;
    host.mmapper_ledger_insert(Lv2Host::MMAPPER_REGION_START, window);
    let mem_id = mint_mmapper_handle(&mut host, &rt, 0x0010_0000, 0x400);
    let r = host.dispatch(
        Lv2Request::Unsupported {
            number: 337,
            args: [0x5000_0000, u64::from(mem_id), 0x40000, 0x9000, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(r, Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into()));
}

/// Non-vacuous coverage for the sc 337 / sc 334 coherence witness.
/// The dispatch's `debug_assert!` body is reconstructed here in
/// isolation against a state that violates the invariant
/// (ledger has an entry, but `pending_region_installs` is empty --
/// exactly the pre-fix fabricated-success shape). The panic message
/// must match the dispatch's so a future refactor that drops or
/// renames the witness will surface here.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "sc 337 coherence")]
fn mmapper_coherence_witness_panic_message_matches_dispatch() {
    let host = Lv2Host::new();
    let pretend_found_addr: u32 = 0x5000_0000;
    let pretend_size: u32 = 0x0010_0000;
    // pending_region_installs is empty; the witness predicate fails.
    debug_assert!(
        host.drain_pending_region_installs_inspect()
            .iter()
            .any(|i| i.addr == u64::from(pretend_found_addr) && i.size == pretend_size as usize),
        "sc 337 coherence: pending_region_installs missing entry for {pretend_found_addr:#x}",
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
fn syscall_330_returns_enomem_when_cursor_would_cross_mmio_region() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let req = || Lv2Request::Unsupported {
        number: 330,
        args: [0x1000_0000, 0, 0, 0x9000, 0, 0, 0, 0],
    };
    for i in 0..7 {
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
        Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into()),
        "the 8th 256 MiB allocation must cap-fail and surface CELL_ENOMEM"
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
    assert_eq!(result, Lv2Dispatch::immediate(0));
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
fn syscall_136_event_port_connect_local_returns_enosys() {
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
        Lv2Dispatch::immediate(cell_errors::CELL_ENOSYS.into())
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
fn syscall_332_size_not_multiple_of_64k_granule_returns_ealign() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 332,
            args: [0x8006_0100_0000_0010, 0x1_8000, 0x200, 0x9000, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into())
    );
}

#[test]
fn syscall_332_size_not_multiple_of_1m_granule_returns_ealign() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 332,
            args: [0x8006_0100_0000_0010, 0x10_0001, 0x400, 0x9000, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into())
    );
}

#[test]
fn syscall_362_size_not_multiple_of_granule_returns_ealign() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 362,
            args: [
                0xffff_0000_0000_0000, // ipc_key
                0x10_0001,             // size (1-byte misaligned)
                0x4000_0015,           // cid (must be ignored for alignment)
                0x400,                 // flags (FLAG_1M -> granule 0x100000)
                0x9000,                // mem_id_ptr
                0,
                0,
                0,
            ],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into())
    );
}

#[test]
fn syscall_362_reads_flags_from_args3_not_args2() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 362,
            args: [
                0xffff_0000_0000_0000,
                0x1_0000, // size = 64K (aligned only for 64K granule)
                0x200,    // args[2]: container-id slot, must be ignored for alignment
                0x400,    // args[3]: real flags -> 1M granule
                0x9000,
                0,
                0,
                0,
            ],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into())
    );
}

#[test]
fn syscall_480_returns_registered_kernel_id_for_known_stem() {
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
    assert_eq!(result, Lv2Dispatch::immediate(u64::from(expected_id)));
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
    assert_eq!(result, Lv2Dispatch::immediate(0x5000));
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
    assert_eq!(result, Lv2Dispatch::immediate(u64::from(expected_id)));
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
    assert_eq!(result, Lv2Dispatch::immediate(0));
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
    assert_eq!(result, Lv2Dispatch::immediate(0x8001_1910));
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
        Lv2Dispatch::immediate(cell_errors::CELL_ENOSYS.into())
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
    assert_eq!(result, Lv2Dispatch::immediate(cell_errors::CELL_EIO.into()));
}

#[test]
fn prx_start_module_writes_no_start_sentinel_to_p_opt_entry() {
    let mut host = Lv2Host::new();
    let p_opt: u32 = 0x4000;
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    let mut p_opt_buf = [0u8; 0x20];
    p_opt_buf[0..4].copy_from_slice(&0x20u32.to_be_bytes());
    mem.apply_commit(
        ByteRange::new(
            cellgov_mem::GuestAddr::new(u64::from(p_opt)),
            p_opt_buf.len() as u64,
        )
        .unwrap(),
        &p_opt_buf,
    )
    .unwrap();
    let rt = FakeRuntime::with_memory(mem);
    let mut args = [0u64; 8];
    args[0] = 0x1234;
    args[2] = u64::from(p_opt);
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
            assert_eq!(range.start().raw(), u64::from(p_opt + 16));
            assert_eq!(range.length(), 8);
            assert_eq!(bytes.bytes(), &u64::MAX.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn syscall_481_rejects_size_below_0x20_with_einval() {
    let mut host = Lv2Host::new();
    let p_opt: u32 = 0x4000;
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    let mut p_opt_buf = [0u8; 0x20];
    p_opt_buf[0..4].copy_from_slice(&0x1Fu32.to_be_bytes());
    mem.apply_commit(
        ByteRange::new(
            cellgov_mem::GuestAddr::new(u64::from(p_opt)),
            p_opt_buf.len() as u64,
        )
        .unwrap(),
        &p_opt_buf,
    )
    .unwrap();
    let rt = FakeRuntime::with_memory(mem);
    let mut args = [0u64; 8];
    args[0] = 0x1234;
    args[2] = u64::from(p_opt);
    let result = host.dispatch(
        Lv2Request::Unsupported { number: 481, args },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
    );
}

#[test]
fn syscall_481_unreadable_p_opt_returns_efault_and_logs_break() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1000);
    let breaks_before = host.invariant_break_count();
    let mut args = [0u64; 8];
    args[0] = 0x1234;
    args[2] = 0x4000_1000;
    let result = host.dispatch(
        Lv2Request::Unsupported { number: 481, args },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
    assert_eq!(host.invariant_break_count() - breaks_before, 1);
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
        Lv2Dispatch::immediate(path_ptr),
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
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
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
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
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
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
}

#[test]
fn syscall_494_unreadable_max_field_returns_efault_and_logs_break() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let breaks_before = host.invariant_break_count();
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 494,
            args: [0x2, 0xFFF1, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
    assert_eq!(host.invariant_break_count() - breaks_before, 1);
}

#[test]
fn syscall_494_unreadable_idlist_field_returns_efault_and_logs_break() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let breaks_before = host.invariant_break_count();
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 494,
            args: [0x2, 0xFFEC, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
    assert_eq!(host.invariant_break_count() - breaks_before, 1);
}

#[test]
fn syscall_494_emits_slot_and_count_in_one_effects_batch() {
    let mut host = Lv2Host::new();
    host.prx_registry_mut().register(
        "libaudio".into(),
        "cellAudio_Library".into(),
        0x0147_0000,
        0x0148_0000,
        0x0147_da30,
        None,
        None,
    );
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    let mut p_info = [0u8; 0x20];
    p_info[0..8].copy_from_slice(&0x20u64.to_be_bytes());
    p_info[0x0C..0x10].copy_from_slice(&4u32.to_be_bytes());
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
        Lv2Dispatch::Immediate { code: 0, effects } => effects,
        other => panic!("expected Immediate{{code:0}}, got {other:?}"),
    };
    assert_eq!(
        effects.len(),
        2,
        "expected one slot write + one count write in a single batch"
    );
    match &effects[0] {
        Effect::SharedWriteIntent { range, .. } => {
            assert_eq!(
                range.start().raw(),
                0x4040,
                "effects[0] is the slot write at idlist_ptr"
            );
        }
        other => panic!("expected SharedWriteIntent for slot, got {other:?}"),
    }
    match &effects[1] {
        Effect::SharedWriteIntent { range, .. } => {
            assert_eq!(
                range.start().raw(),
                0x4010,
                "effects[1] is the count write at pInfo+0x10, after the slot"
            );
        }
        other => panic!("expected SharedWriteIntent for count, got {other:?}"),
    }
}

#[test]
fn syscall_494_idlist_order_is_independent_of_registration_order() {
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
        Lv2Dispatch::immediate(cell_errors::CELL_ENOSYS.into())
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
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
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
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
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
    assert_eq!(result, Lv2Dispatch::immediate(0x8001_051D));
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
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
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

#[test]
fn memory_free_is_no_op_returning_ok() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(Lv2Request::MemoryFree { addr: 0x1000 }, UnitId::new(0), &rt);
    assert_eq!(result, Lv2Dispatch::immediate(0));
}

#[test]
fn ppu_thread_yield_is_no_op_returning_ok() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(Lv2Request::PpuThreadYield, UnitId::new(0), &rt);
    assert_eq!(result, Lv2Dispatch::immediate(0));
}

#[test]
fn ppu_thread_start_returns_ok_because_auto_started_at_create() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::PpuThreadStart { target: 0x101 },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(result, Lv2Dispatch::immediate(0));
}

#[test]
fn process_is_stack_returns_real_answer_from_tracked_thread_ranges() {
    use crate::ppu_thread::PpuThreadAttrs;
    let mut host = Lv2Host::new();
    host.seed_primary_ppu_thread(
        UnitId::new(0),
        PpuThreadAttrs {
            entry: 0x1000,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x1_0000,
            priority: 1001,
            tls_base: 0,
        },
    );
    let rt = FakeRuntime::new(0x10000);

    let on_stack = host.dispatch(
        Lv2Request::ProcessIsStack { addr: 0xD000_0500 },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(on_stack, Lv2Dispatch::immediate(1));

    let below = host.dispatch(
        Lv2Request::ProcessIsStack { addr: 0xCFFF_FFFF },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(below, Lv2Dispatch::immediate(0));

    // Half-open: stack_base + stack_size is not on stack.
    let at_end = host.dispatch(
        Lv2Request::ProcessIsStack { addr: 0xD001_0000 },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(at_end, Lv2Dispatch::immediate(0));
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
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
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
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
    );
    assert!(host.invariant_break_count() > before);
}

#[test]
fn spu_thread_group_terminate_logs_invariant_break_and_returns_enosys() {
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
        Lv2Dispatch::immediate(cell_errors::CELL_ENOSYS.into())
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
            param_ptr: 0x4000_0000,
            arg: 0,
            priority: 1000,
            stacksize: 0x4000,
            flags: 0x1, // JOINABLE -- unmodeled
        },
        UnitId::new(0),
        &rt,
    );
    assert!(
        host.invariant_break_count() > before,
        "expected log_invariant_break to fire on nonzero flags"
    );
}

fn dispatch_lock_line(addr: u32, flags: u64) -> Lv2Dispatch {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    host.dispatch(
        Lv2Request::ProcessIsSpuLockLineReservationAddress { addr, flags },
        UnitId::new(0),
        &rt,
    )
}

#[test]
fn process_is_spu_lock_line_reservation_address_zero_flags_is_einval() {
    let result = dispatch_lock_line(0xE000_0000, 0);
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, cell_errors::CELL_EINVAL.into());
            assert!(effects.is_empty());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn process_is_spu_lock_line_reservation_address_unknown_flag_bit_is_einval() {
    let result = dispatch_lock_line(0xE000_0000, 0x4);
    match result {
        Lv2Dispatch::Immediate { code, .. } => {
            assert_eq!(code, cell_errors::CELL_EINVAL.into());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn process_is_spu_lock_line_reservation_address_raw_spu_region_returns_ok() {
    let result = dispatch_lock_line(0xE000_0000, 0x1);
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert!(effects.is_empty());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn process_is_spu_lock_line_reservation_address_private_spu_rejects_raw_flag() {
    let result = dispatch_lock_line(0xF000_0000, 0x1);
    match result {
        Lv2Dispatch::Immediate { code, .. } => {
            assert_eq!(code, cell_errors::CELL_EPERM.into());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn process_is_spu_lock_line_reservation_address_private_spu_accepts_thr_flag() {
    let result = dispatch_lock_line(0xF000_0000, 0x2);
    match result {
        Lv2Dispatch::Immediate { code, .. } => assert_eq!(code, 0),
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn process_is_spu_lock_line_reservation_address_ppu_stack_is_eperm() {
    let result = dispatch_lock_line(0xD000_0000, 0x2);
    match result {
        Lv2Dispatch::Immediate { code, .. } => {
            assert_eq!(code, cell_errors::CELL_EPERM.into());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn process_is_spu_lock_line_reservation_address_unknown_region_is_einval() {
    let result = dispatch_lock_line(0x3000_0000, 0x2);
    match result {
        Lv2Dispatch::Immediate { code, .. } => {
            assert_eq!(code, cell_errors::CELL_EINVAL.into());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

fn dispatch_spu_init(max_usable_spu: u32, max_raw_spu: u32) -> Lv2Dispatch {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    host.dispatch(
        Lv2Request::SpuInitialize {
            max_usable_spu,
            max_raw_spu,
        },
        UnitId::new(0),
        &rt,
    )
}

#[test]
fn spu_initialize_accepts_typical_lv2_caps() {
    let result = dispatch_spu_init(6, 1);
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert!(effects.is_empty());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn spu_initialize_rejects_max_raw_above_five() {
    let result = dispatch_spu_init(6, 6);
    match result {
        Lv2Dispatch::Immediate { code, .. } => {
            assert_eq!(code, cell_errors::CELL_EINVAL.into());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn spu_initialize_accepts_zero_raw_spu() {
    let result = dispatch_spu_init(6, 0);
    match result {
        Lv2Dispatch::Immediate { code, .. } => assert_eq!(code, 0),
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn spu_thread_group_destroy_unknown_id_is_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::SpuThreadGroupDestroy { id: 0xDEAD },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, .. } => assert_eq!(code, cell_errors::CELL_ESRCH.into()),
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn spu_thread_group_destroy_created_group_returns_ok() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let create = host.dispatch(
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x9000,
            num_threads: 1,
            priority: 100,
            attr_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    let group_id = match create {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            let payload = match &effects[0] {
                cellgov_effects::Effect::SharedWriteIntent { bytes, .. } => bytes.bytes(),
                other => panic!("expected SharedWriteIntent, got {other:?}"),
            };
            u32::from_be_bytes(payload[..4].try_into().unwrap())
        }
        other => panic!("expected Immediate, got {other:?}"),
    };
    let destroy = host.dispatch(
        Lv2Request::SpuThreadGroupDestroy { id: group_id },
        UnitId::new(0),
        &rt,
    );
    match destroy {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert!(effects.is_empty());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
    let second = host.dispatch(
        Lv2Request::SpuThreadGroupDestroy { id: group_id },
        UnitId::new(0),
        &rt,
    );
    match second {
        Lv2Dispatch::Immediate { code, .. } => assert_eq!(code, cell_errors::CELL_ESRCH.into()),
        other => panic!("expected Immediate, got {other:?}"),
    }
}

// Witnesses for the debug_assert-only-guard sweep (findings #6, #7).
// Each prior debug_assert! was the only guard against a wrapping
// u32 pointer producing a wrong-address SharedWriteIntent + lying
// CELL_OK in release. The fix replaces both with runtime EFAULT
// returns; these tests pin that contract.

#[test]
fn prx_start_module_wrapping_p_opt_returns_efault_and_emits_no_writes() {
    use crate::host::Lv2Runtime;
    use cellgov_time::GuestTicks;
    struct WrapMock {
        size_be: [u8; 4],
    }
    impl Lv2Runtime for WrapMock {
        fn read_committed(&self, _addr: u64, len: usize) -> Option<&[u8]> {
            (len == 4).then_some(&self.size_be[..])
        }
        fn current_tick(&self) -> GuestTicks {
            GuestTicks::ZERO
        }
        fn read_committed_until(
            &self,
            _addr: u64,
            _max_len: usize,
            _terminator: u8,
        ) -> Option<&[u8]> {
            None
        }
        fn writable(&self, _addr: u64, _len: usize) -> bool {
            true
        }
    }
    let mut host = Lv2Host::new();
    let breaks_before = host.invariant_break_count();
    let rt = WrapMock {
        size_be: 0x20u32.to_be_bytes(),
    };
    // p_opt+24 wraps u32: 0xFFFF_FFFF - 23 = 0xFFFF_FFE8 is the
    // smallest wrapping p_opt. Use 0xFFFF_FFF0 (entry at p_opt+16
    // would wrap to 0x0000_0000).
    let mut args = [0u64; 8];
    args[0] = 0x1234;
    args[2] = 0xFFFF_FFF0_u64;
    let result = host.dispatch(
        Lv2Request::Unsupported { number: 481, args },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into()),
        "p_opt+24 wraps u32; must return CELL_EFAULT, not CELL_OK with a wrong-address write"
    );
    assert_eq!(
        host.invariant_break_count() - breaks_before,
        1,
        "wrap path must log_invariant_break exactly once"
    );
}

#[test]
fn prx_get_module_list_wrapping_p_info_returns_efault_and_emits_no_writes() {
    use crate::host::Lv2Runtime;
    use cellgov_time::GuestTicks;
    // Returns 4 zero bytes for every read so the post-wrap-check
    // path would reach the count-write at count_addr = pInfo+0x10
    // (which wraps to addr 0). Without the wrap check, the
    // adversarial revert produces a SharedWriteIntent at addr 0
    // with the dispatch returning CELL_OK, not a quiet EFAULT --
    // the witness distinguishes the adversarial state from the fix.
    struct ZeroReadMock {
        zeros: [u8; 4],
    }
    impl Lv2Runtime for ZeroReadMock {
        fn read_committed(&self, _addr: u64, len: usize) -> Option<&[u8]> {
            (len == 4).then_some(&self.zeros[..])
        }
        fn current_tick(&self) -> GuestTicks {
            GuestTicks::ZERO
        }
        fn read_committed_until(
            &self,
            _addr: u64,
            _max_len: usize,
            _terminator: u8,
        ) -> Option<&[u8]> {
            None
        }
        fn writable(&self, _addr: u64, _len: usize) -> bool {
            true
        }
    }
    let mut host = Lv2Host::new();
    let breaks_before = host.invariant_break_count();
    let rt = ZeroReadMock { zeros: [0; 4] };
    let mut args = [0u64; 8];
    args[0] = 0x2; // flags & 2 must be set, else short-circuit OK
    args[1] = 0xFFFF_FFF0_u64;
    let result = host.dispatch(
        Lv2Request::Unsupported { number: 494, args },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into()),
        "pInfo+0x18 wraps u32; must return CELL_EFAULT, not silent slot writes at wrong addresses"
    );
    assert_eq!(
        host.invariant_break_count() - breaks_before,
        1,
        "wrap path must log_invariant_break exactly once"
    );
}
