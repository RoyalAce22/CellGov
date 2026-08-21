//! Memory-container and mmapper syscalls: id minting, granule gates, install search.

use super::*;

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
fn cell_ps3_user_memory_total_is_213_mib() {
    assert_eq!(cellgov_ps3_abi::sys_memory::USER_MEMORY_TOTAL, 0x0D50_0000);
    assert_eq!(cellgov_ps3_abi::sys_memory::USER_MEMORY_TOTAL, 223_346_688);
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

/// The kernel truncates the request to the 1 MiB granule first, so a
/// sub-granule container has nothing left to allocate. Oracle: RPCS3
/// `sys_memory.cpp` `sys_memory_container_create`.
#[test]
fn syscall_324_sub_granule_size_returns_enomem_and_mints_no_id() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    for size in [0u64, 1, 0xF_FFFF] {
        let result = host.dispatch(
            Lv2Request::Unsupported {
                number: 324,
                args: [0x9000, size, 0, 0, 0, 0, 0, 0],
            },
            UnitId::new(0),
            &rt,
        );
        assert_eq!(
            result,
            Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into()),
            "size {size:#x} rounds down to zero",
        );
    }
    assert_eq!(
        host.alloc_id(),
        Lv2Host::new().alloc_id(),
        "a refused create must not consume an id",
    );
}

/// RPCS3 truncates and refuses the size before it ever reaches the
/// `cid` write, so a call wrong in both ways answers for its size.
#[test]
fn syscall_324_sub_granule_size_outranks_a_null_cid() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 324,
            args: [0, 0xF_FFFF, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into())
    );
}

#[test]
fn syscall_324_null_cid_with_an_accepted_size_is_efault() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 324,
            args: [0, 0x10_0000, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
    assert_eq!(
        host.alloc_id(),
        Lv2Host::new().alloc_id(),
        "a refused create must not consume an id",
    );
}

#[test]
fn syscall_324_exactly_one_granule_is_accepted() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 324,
            args: [0x9000, 0x10_0000, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert!(matches!(result, Lv2Dispatch::Immediate { code: 0, .. }));
}

/// A misaligned reservation is refused, not silently rounded up to
/// the next 256 MiB area. Oracle: RPCS3 `sys_mmapper.cpp`
/// `sys_mmapper_allocate_address`.
#[test]
fn syscall_330_size_off_the_area_granule_returns_ealign() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    for size in [1u64, 0x1000_0001, 0x0FFF_FFFF] {
        let result = host.dispatch(
            Lv2Request::Unsupported {
                number: 330,
                args: [size, 0x400, 0, 0x9000, 0, 0, 0, 0],
            },
            UnitId::new(0),
            &rt,
        );
        assert_eq!(
            result,
            Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into()),
            "size {size:#x} is not an area multiple",
        );
    }
}

/// A `size` past `u32::MAX` must not narrow into an accepted request.
#[test]
fn syscall_330_size_beyond_u32_returns_enomem() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 330,
            args: [0x1_1000_0000, 0x400, 0, 0x9000, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into()),
        "the low word alone must not decide the request",
    );
}

#[test]
fn syscall_330_rejects_an_alignment_outside_the_area_sizes() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    for alignment in [0x1000u64, 0x10_0000, 0x3000_0000, 0x1_0000_0000] {
        let result = host.dispatch(
            Lv2Request::Unsupported {
                number: 330,
                args: [0x1000_0000, 0x400, alignment, 0x9000, 0, 0, 0, 0],
            },
            UnitId::new(0),
            &rt,
        );
        assert_eq!(
            result,
            Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into()),
            "alignment {alignment:#x} is not a VM area size",
        );
    }
}

/// RPCS3 checks `size`, then `alignment`, and only reaches the
/// `alloc_addr` write afterwards, so a call wrong in two ways answers
/// for its arguments first.
#[test]
fn syscall_330_argument_refusals_outrank_a_null_alloc_addr() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    // Bad size, null out-pointer.
    assert_eq!(
        host.dispatch(
            Lv2Request::Unsupported {
                number: 330,
                args: [1, 0x400, 0, 0, 0, 0, 0, 0],
            },
            UnitId::new(0),
            &rt,
        ),
        Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into()),
    );
    // Bad alignment, null out-pointer.
    assert_eq!(
        host.dispatch(
            Lv2Request::Unsupported {
                number: 330,
                args: [0x1000_0000, 0x400, 0x1000, 0, 0, 0, 0, 0],
            },
            UnitId::new(0),
            &rt,
        ),
        Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into()),
    );
    // Everything valid but the out-pointer.
    assert_eq!(
        host.dispatch(
            Lv2Request::Unsupported {
                number: 330,
                args: [0x1000_0000, 0x400, 0, 0, 0, 0, 0, 0],
            },
            UnitId::new(0),
            &rt,
        ),
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into()),
    );
}

/// A zero alignment is the documented PSL1GHT allowance: the kernel
/// reads it as the default area size.
#[test]
fn syscall_330_accepts_a_zero_alignment_and_the_four_area_sizes() {
    let rt = FakeRuntime::new(0x10000);
    for alignment in [0u64, 0x1000_0000, 0x2000_0000, 0x4000_0000, 0x8000_0000] {
        let mut host = Lv2Host::new();
        let result = host.dispatch(
            Lv2Request::Unsupported {
                number: 330,
                args: [0x1000_0000, 0x400, alignment, 0x9000, 0, 0, 0, 0],
            },
            UnitId::new(0),
            &rt,
        );
        assert!(
            matches!(result, Lv2Dispatch::Immediate { code: 0, .. }),
            "alignment {alignment:#x}: got {result:?}",
        );
    }
}

#[test]
fn syscall_334_unknown_mem_id_returns_esrch_and_logs_break() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let breaks_before = host.observability().invariant_break_count;
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
    assert_eq!(
        host.observability().invariant_break_count - breaks_before,
        1
    );
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
    assert_eq!(installs, vec![(0x5000_0000_u64, 0x0400_0000_usize, None)]);
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
    assert_eq!(installs, vec![(0x5400_0000_u64, 0x00a0_0000_usize, None)]);
}

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
    assert_eq!(installs, vec![(0x5000_0000_u64, 0x0010_0000_usize, None)]);
}

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

#[test]
fn syscall_337_unknown_mem_id_returns_esrch_and_logs_break() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let breaks_before = host.observability().invariant_break_count;
    let r = host.dispatch(
        Lv2Request::Unsupported {
            number: 337,
            args: [0x5000_0000, 0xdead_beef, 0x40000, 0x9000, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(r, Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into()));
    assert_eq!(
        host.observability().invariant_break_count - breaks_before,
        1
    );
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
    // Fill the entire mmapper window in the ledger the search consults.
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

/// Non-vacuous coverage for the sc 337 / sc 334 coherence witness: the
/// dispatch's `debug_assert!` body reconstructed against a state that
/// violates it (a ledger entry with `pending_region_installs` empty),
/// with the same panic message the dispatch carries.
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
fn syscall_332_writes_fresh_mem_id_to_mem_id_ptr() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let first_call = host.dispatch(
        Lv2Request::Unsupported {
            number: 332,
            args: [0xffff_0000_0000_0000, 0x10000, 0x200, 0x9000, 0, 0, 0, 0],
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
            args: [0xffff_0000_0000_0000, 0x10000, 0x200, 0x9100, 0, 0, 0, 0],
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
        "successive keyless mem_ids must be monotonic: first=0x{first_id:x} second=0x{second_id:x}"
    );
}

/// Run sc 332 with `ipc_key` and return the mem_id written to `*0x9000`.
fn dispatch_332_keyed(host: &mut Lv2Host, rt: &FakeRuntime, ipc_key: u64, size: u64) -> u32 {
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 332,
            args: [ipc_key, size, 0x200, 0x9000, 0, 0, 0, 0],
        },
        UnitId::new(0),
        rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = result else {
        panic!("332 must succeed, got {result:?}");
    };
    let Effect::SharedWriteIntent { bytes, .. } = &effects[0] else {
        panic!("332 must emit a SharedWriteIntent");
    };
    u32::from_be_bytes(bytes.bytes().try_into().unwrap())
}

#[test]
fn syscall_332_keyed_create_registers_ipc_entry() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let mem_id = dispatch_332_keyed(&mut host, &rt, 0x8006_0100_0000_0010, 0x10000);
    assert_eq!(
        host.mmapper_ipc().get(&0x8006_0100_0000_0010),
        Some(&mem_id)
    );
}

#[test]
fn syscall_332_same_ipc_key_returns_existing_mem_id() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let first = dispatch_332_keyed(&mut host, &rt, 0x8006_0100_0000_0010, 0x10000);
    // NOT_CARE: size differs but the existing handle is returned with
    // its original size intact.
    let second = dispatch_332_keyed(&mut host, &rt, 0x8006_0100_0000_0010, 0x20000);
    assert_eq!(first, second);
    assert_eq!(host.mmapper_ipc().len(), 1);
    let handle = host
        .state
        .mmapper_handles
        .get(first)
        .expect("handle must exist");
    assert_eq!(handle.size, 0x10000);
}

#[test]
fn syscall_332_distinct_ipc_keys_mint_distinct_mem_ids() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let first = dispatch_332_keyed(&mut host, &rt, 0x8006_0100_0000_0010, 0x10000);
    let second = dispatch_332_keyed(&mut host, &rt, 0x8006_0100_0000_0020, 0x10000);
    assert_ne!(first, second);
    assert_eq!(host.mmapper_ipc().len(), 2);
}

#[test]
fn syscall_337_applies_registered_seed_on_first_map_of_keyed_shm() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    host.register_system_seed(crate::SystemStateSeed {
        shm_ipc_key: 0x8006_0100_0000_0010,
        writes: vec![(0, vec![0xAA; 4]), (0x8000, vec![0xBB])],
    });
    let mem_id = dispatch_332_keyed(&mut host, &rt, 0x8006_0100_0000_0010, 0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 337,
            args: [0x5000_0000, u64::from(mem_id), 0, 0x9100, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = result else {
        panic!("337 must succeed, got {result:?}");
    };
    assert_eq!(effects.len(), 3, "two seed writes + alloc_addr write-back");
    let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] else {
        panic!("seed write must be a SharedWriteIntent");
    };
    assert_eq!(range.start().raw(), 0x5000_0000);
    assert_eq!(bytes.bytes(), [0xAA; 4]);
    let Effect::SharedWriteIntent { range, bytes, .. } = &effects[1] else {
        panic!("seed write must be a SharedWriteIntent");
    };
    assert_eq!(range.start().raw(), 0x5000_8000);
    assert_eq!(bytes.bytes(), [0xBB]);
    let Effect::SharedWriteIntent { range, .. } = &effects[2] else {
        panic!("alloc_addr write-back must be a SharedWriteIntent");
    };
    assert_eq!(range.start().raw(), 0x9100);
    assert!(host.system_seed_applied(0x8006_0100_0000_0010));
}

#[test]
fn syscall_337_second_map_does_not_reapply_seed() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    host.register_system_seed(crate::SystemStateSeed {
        shm_ipc_key: 0x8006_0100_0000_0010,
        writes: vec![(0, vec![0xAA; 4])],
    });
    let mem_id = dispatch_332_keyed(&mut host, &rt, 0x8006_0100_0000_0010, 0x10000);
    for _ in 0..2 {
        host.dispatch(
            Lv2Request::Unsupported {
                number: 337,
                args: [0x5000_0000, u64::from(mem_id), 0, 0x9100, 0, 0, 0, 0],
            },
            UnitId::new(0),
            &rt,
        );
    }
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 337,
            args: [0x5000_0000, u64::from(mem_id), 0, 0x9100, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = result else {
        panic!("337 must succeed, got {result:?}");
    };
    assert_eq!(effects.len(), 1, "re-map must emit only the write-back");
}

#[test]
fn syscall_334_applies_registered_seed_at_fixed_addr() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    host.register_system_seed(crate::SystemStateSeed {
        shm_ipc_key: 0x8006_0100_0000_0010,
        writes: vec![(0x40, vec![0xCC; 8])],
    });
    let mem_id = dispatch_332_keyed(&mut host, &rt, 0x8006_0100_0000_0010, 0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 334,
            args: [0x5000_0000, u64::from(mem_id), 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = result else {
        panic!("334 must succeed, got {result:?}");
    };
    assert_eq!(effects.len(), 1);
    let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] else {
        panic!("seed write must be a SharedWriteIntent");
    };
    assert_eq!(range.start().raw(), 0x5000_0040);
    assert_eq!(bytes.bytes(), [0xCC; 8]);
}

#[test]
fn syscall_337_keyless_shm_ignores_registered_seeds() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    host.register_system_seed(crate::SystemStateSeed {
        shm_ipc_key: 0x8006_0100_0000_0010,
        writes: vec![(0, vec![0xAA; 4])],
    });
    let mem_id = dispatch_332_keyed(&mut host, &rt, 0xffff_0000_0000_0000, 0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 337,
            args: [0x5000_0000, u64::from(mem_id), 0, 0x9100, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = result else {
        panic!("337 must succeed, got {result:?}");
    };
    assert_eq!(
        effects.len(),
        1,
        "keyless shm must emit only the write-back"
    );
    assert!(!host.system_seed_applied(0x8006_0100_0000_0010));
}

#[test]
fn syscall_332_sentinel_and_zero_keys_bypass_registration() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let sentinel_a = dispatch_332_keyed(&mut host, &rt, 0xffff_0000_0000_0000, 0x10000);
    let sentinel_b = dispatch_332_keyed(&mut host, &rt, 0xffff_0000_0000_0000, 0x10000);
    let zero_a = dispatch_332_keyed(&mut host, &rt, 0, 0x10000);
    let zero_b = dispatch_332_keyed(&mut host, &rt, 0, 0x10000);
    assert_ne!(sentinel_a, sentinel_b);
    assert_ne!(zero_a, zero_b);
    assert!(host.mmapper_ipc().is_empty());
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
fn memory_free_is_no_op_returning_ok() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(Lv2Request::MemoryFree { addr: 0x1000 }, UnitId::new(0), &rt);
    assert_eq!(result, Lv2Dispatch::immediate(0));
}

#[test]
fn syscall_334_over_an_occupied_window_returns_ebusy_and_stages_nothing() {
    let mut host = Lv2Host::new();
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    mem.install_region(
        0x5000_0000,
        0x10000,
        "loader",
        cellgov_mem::PageSize::Page64K,
    )
    .unwrap();
    let rt = FakeRuntime::with_memory(mem);
    let mem_id = mint_mmapper_handle(&mut host, &rt, 0x10000, 0x200);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 334,
            args: [0x5000_0000, u64::from(mem_id), 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into()),
        "an occupied window is EBUSY, not a fabricated OK",
    );
    let installs: Vec<_> = host.drain_pending_region_installs().collect();
    assert!(installs.is_empty(), "a refused map must stage no install");
}

#[test]
fn syscall_334_partial_overlap_is_ebusy_too() {
    let mut host = Lv2Host::new();
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    mem.install_region(
        0x5000_8000,
        0x10000,
        "loader",
        cellgov_mem::PageSize::Page64K,
    )
    .unwrap();
    let rt = FakeRuntime::with_memory(mem);
    let mem_id = mint_mmapper_handle(&mut host, &rt, 0x10000, 0x200);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 334,
            args: [0x5000_0000, u64::from(mem_id), 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into())
    );
}

/// RPCS3's `area->alloc` searches the caller's vm area, so an
/// occupied window is skipped rather than returned and then refused.
#[test]
fn syscall_337_search_skips_caller_occupied_windows() {
    let mut host = Lv2Host::new();
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    mem.install_region(
        u64::from(Lv2Host::MMAPPER_REGION_START),
        0x10000,
        "loader",
        cellgov_mem::PageSize::Page64K,
    )
    .unwrap();
    let rt = FakeRuntime::with_memory(mem);
    let mem_id = mint_mmapper_handle(&mut host, &rt, 0x10000, 0x200);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 337,
            args: [
                u64::from(Lv2Host::MMAPPER_REGION_START),
                u64::from(mem_id),
                0,
                0x9100,
                0,
                0,
                0,
                0,
            ],
        },
        UnitId::new(0),
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = result else {
        panic!("337 must succeed by skipping the occupied window, got {result:?}");
    };
    let Effect::SharedWriteIntent { bytes, .. } = &effects[0] else {
        panic!("337 must write the found address back");
    };
    assert_eq!(
        u32::from_be_bytes(bytes.bytes().try_into().unwrap()),
        Lv2Host::MMAPPER_REGION_START + 0x10000,
        "the search must land past the caller-occupied window",
    );
}

/// The host ledger records a map the runtime has not committed yet,
/// and outlives an install the drain refused, so 334 must consult it
/// as well as the caller's committed layout.
#[test]
fn syscall_334_over_a_window_the_host_already_mapped_returns_ebusy() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let first = mint_mmapper_handle(&mut host, &rt, 0x10_0000, 0x400);
    let second = mint_mmapper_handle(&mut host, &rt, 0x10_0000, 0x400);
    let map = |host: &mut Lv2Host, mem_id: u32, addr: u64| {
        host.dispatch(
            Lv2Request::Unsupported {
                number: 334,
                args: [addr, u64::from(mem_id), 0, 0, 0, 0, 0, 0],
            },
            UnitId::new(0),
            &rt,
        )
    };
    assert_eq!(
        map(&mut host, first, 0x5000_0000),
        Lv2Dispatch::immediate(0)
    );
    assert_eq!(
        map(&mut host, second, 0x5000_0000),
        Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into()),
        "re-mapping a window the ledger already holds is EBUSY",
    );
    let installs: Vec<_> = host.drain_pending_region_installs().collect();
    assert_eq!(installs.len(), 1, "the refused map must stage no install");
}

#[test]
fn syscall_334_partially_overlapping_a_prior_337_map_returns_ebusy() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let searched = mint_mmapper_handle(&mut host, &rt, 0x20_0000, 0x400);
    let fixed = mint_mmapper_handle(&mut host, &rt, 0x10_0000, 0x400);
    // 337 claims [0x5000_0000, 0x5020_0000).
    host.dispatch(
        Lv2Request::Unsupported {
            number: 337,
            args: [0x5000_0000, u64::from(searched), 0, 0x9000, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    // A granule-aligned 334 whose window starts inside that range.
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 334,
            args: [0x5010_0000, u64::from(fixed), 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into())
    );
}

/// The gates keep the ledger non-overlapping, so a nested entry is not
/// reachable through 334 / 337; seeding one directly pins that the busy
/// test surveys every entry rather than the nearest one.
#[test]
fn syscall_334_over_a_window_nested_in_a_longer_ledger_entry_returns_ebusy() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let handle = mint_mmapper_handle(&mut host, &rt, 0x10_0000, 0x400);
    // A long entry, then a shorter one starting later and ending
    // before the candidate -- the shorter is what `next_back` finds.
    host.mmapper_ledger_insert(0x5000_0000, 0x40_0000);
    host.mmapper_ledger_insert(0x5010_0000, 0x1_0000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 334,
            args: [0x5020_0000, u64::from(handle), 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into()),
        "a window covered only by the longer, earlier entry is still busy",
    );
}

#[test]
fn syscall_334_immediately_after_a_prior_map_is_not_busy() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let first = mint_mmapper_handle(&mut host, &rt, 0x10_0000, 0x400);
    let second = mint_mmapper_handle(&mut host, &rt, 0x10_0000, 0x400);
    let map = |host: &mut Lv2Host, mem_id: u32, addr: u64| {
        host.dispatch(
            Lv2Request::Unsupported {
                number: 334,
                args: [addr, u64::from(mem_id), 0, 0, 0, 0, 0, 0],
            },
            UnitId::new(0),
            &rt,
        )
    };
    assert_eq!(
        map(&mut host, first, 0x5000_0000),
        Lv2Dispatch::immediate(0)
    );
    assert_eq!(
        map(&mut host, second, 0x5010_0000),
        Lv2Dispatch::immediate(0),
        "an abutting window shares no byte with the prior map",
    );
}

/// The kernel refuses a zero-size shm outright; rounding it into a
/// zero-size handle would stage a zero-byte region install downstream.
/// Oracle: RPCS3 `sys_mmapper.cpp`
/// `sys_mmapper_allocate_shared_memory`.
#[test]
fn syscall_332_zero_size_returns_ealign_and_mints_no_handle() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 332,
            args: [0x8006_0100_0000_0010, 0, 0x200, 0x9000, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into())
    );
    assert!(host.state.mmapper_handles.is_empty());
    assert!(
        host.mmapper_ipc().is_empty(),
        "a refused create registers no key"
    );
}

#[test]
fn syscall_362_zero_size_returns_ealign() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 362,
            args: [
                0xffff_0000_0000_0000,
                0, // size
                0x4000_0015,
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
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into())
    );
    assert!(host.state.mmapper_handles.is_empty());
}

/// The granularity field is an encoding: a value outside
/// {unset, 64K, 1M} is refused rather than resolved to a
/// granule. Oracle: RPCS3 `sys_mmapper.cpp`
/// `sys_mmapper_allocate_shared_memory` default arm.
#[test]
fn syscall_332_unencodable_granularity_field_returns_einval() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    for flags in [0x600u64, 0x100, 0x800, 0xf00] {
        let result = host.dispatch(
            Lv2Request::Unsupported {
                number: 332,
                args: [0xffff_0000_0000_0000, 0x10_0000, flags, 0x9000, 0, 0, 0, 0],
            },
            UnitId::new(0),
            &rt,
        );
        assert_eq!(
            result,
            Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()),
            "flags {flags:#x} must not resolve to a granule",
        );
    }
    assert!(host.state.mmapper_handles.is_empty());
}

#[test]
fn syscall_362_unencodable_granularity_field_returns_einval() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 362,
            args: [
                0xffff_0000_0000_0000,
                0x10_0000,
                0x4000_0015,
                0x600, // both granularity bits set
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
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
    );
}

/// Bits above the granularity field are not part of the encoding.
#[test]
fn syscall_332_accepts_a_valid_granularity_beside_unrelated_flag_bits() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 332,
            args: [
                0xffff_0000_0000_0000,
                0x1_0000,
                0x4000_0200,
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
    assert!(
        matches!(result, Lv2Dispatch::Immediate { code: 0, .. }),
        "got {result:?}",
    );
}

/// RPCS3 refuses `size` and `flags` before it ever reaches the
/// `mem_id` write, so a call wrong in two ways answers for its
/// arguments first.
#[test]
fn syscall_332_argument_refusals_outrank_a_null_mem_id() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let call = |host: &mut Lv2Host, size: u64, flags: u64| {
        host.dispatch(
            Lv2Request::Unsupported {
                number: 332,
                args: [0xffff_0000_0000_0000, size, flags, 0, 0, 0, 0, 0],
            },
            UnitId::new(0),
            &rt,
        )
    };
    assert_eq!(
        call(&mut host, 0, 0x200),
        Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into()),
    );
    assert_eq!(
        call(&mut host, 0x10_0000, 0x600),
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()),
    );
    assert_eq!(
        call(&mut host, 0x1_0000, 0x400),
        Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into()),
    );
    assert_eq!(
        call(&mut host, 0x10_0000, 0x400),
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into()),
        "only a request the kernel would have accepted reaches the pointer gate",
    );
    assert!(
        host.state.mmapper_handles.is_empty(),
        "no refusal path may mint a handle",
    );
}

#[test]
fn syscall_362_argument_refusals_outrank_a_null_mem_id() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let call = |host: &mut Lv2Host, size: u64, flags: u64| {
        host.dispatch(
            Lv2Request::Unsupported {
                number: 362,
                args: [0xffff_0000_0000_0000, size, 0x4000_0015, flags, 0, 0, 0, 0],
            },
            UnitId::new(0),
            &rt,
        )
    };
    assert_eq!(
        call(&mut host, 0, 0x400),
        Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into()),
    );
    assert_eq!(
        call(&mut host, 0x10_0000, 0x600),
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()),
    );
    assert_eq!(
        call(&mut host, 0x10_0000, 0x400),
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into()),
    );
    assert!(host.state.mmapper_handles.is_empty());
}

/// An unset granularity field means 1 MiB, so a 64 KiB size is
/// misaligned rather than accepted.
#[test]
fn syscall_332_unset_granularity_field_means_the_1m_granule() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 332,
            args: [0xffff_0000_0000_0000, 0x1_0000, 0, 0x9000, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into())
    );
}
