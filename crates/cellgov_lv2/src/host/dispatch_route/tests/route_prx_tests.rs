//! PRX module syscalls: id resolution, module list walking, and p_opt/p_info gates.

use super::*;

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
