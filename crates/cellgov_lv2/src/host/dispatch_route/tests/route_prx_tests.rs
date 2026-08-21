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
fn syscall_480_non_firmware_unknown_path_returns_enoent() {
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
        Lv2Dispatch::immediate(cellgov_ps3_abi::cell_errors::CELL_ENOENT.into())
    );
    assert_eq!(host.observability().prx_load_not_found_count, 1);

    // A path the resolver refused must not start: ESRCH, so the
    // sentinel write is never reached.
    let p_opt: u32 = 0x4000;
    let rt = runtime_with(p_opt, &start_stop_option(0x20, 1, 0));
    let start = start_module(&mut host, 0x5000, p_opt, &rt);
    assert_eq!(
        start,
        Lv2Dispatch::immediate(cellgov_ps3_abi::cell_errors::CELL_ESRCH.into())
    );
}

#[test]
fn syscall_480_firmware_miss_registers_stub_and_start_reaches_sentinel() {
    let mut host = Lv2Host::new();
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    // libmedi ships in retail firmware but is absent from this host's
    // (empty) corpus, so the miss path stubs it.
    let path = b"/dev_flash/sys/external/libmedi.sprx\0";
    mem.apply_commit(
        cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0x4800), path.len() as u64)
            .unwrap(),
        path,
    )
    .unwrap();
    let rt = FakeRuntime::with_memory(mem);
    let load = |host: &mut Lv2Host, rt: &FakeRuntime| {
        host.dispatch(
            Lv2Request::Unsupported {
                number: 480,
                args: [0x4800, 0, 0, 0, 0, 0, 0, 0],
            },
            UnitId::new(0),
            rt,
        )
    };

    let id = match load(&mut host, &rt) {
        Lv2Dispatch::Immediate { code, effects } => {
            assert!(effects.is_empty());
            assert!(
                code >= u64::from(crate::prx_registry::FIRST_KERNEL_ID),
                "stub id {code:#x} must come from the kernel-id space, \
                 not echo the path pointer"
            );
            u32::try_from(code).unwrap()
        }
        other => panic!("expected Immediate, got {other:?}"),
    };
    assert_eq!(host.observability().prx_load_hle_stub_count, 1);

    // Re-load resolves the stub by stem: same id, no second mint.
    assert_eq!(load(&mut host, &rt), Lv2Dispatch::immediate(u64::from(id)));
    assert_eq!(host.observability().prx_load_hle_stub_count, 1);

    // The stub id starts like any loaded module: NO_ENTRY sentinel.
    let p_opt: u32 = 0x4000;
    let rt = runtime_with(p_opt, &start_stop_option(0x20, 1, 0));
    let start = start_module(&mut host, id, p_opt, &rt);
    match start {
        Lv2Dispatch::Immediate { code: 0, effects } => assert_eq!(effects.len(), 1),
        other => panic!("expected Immediate{{code:0}} with sentinel write, got {other:?}"),
    }
}

/// The miss-stub path is gated on the retail firmware module set: a
/// made-up name under `/dev_flash/sys/external/` names nothing any
/// console serves, so minting a success id for it would fabricate.
/// RPCS3's whitelist gate produces the same ENOENT.
#[test]
fn syscall_480_unknown_firmware_name_returns_enoent_not_a_stub() {
    let mut host = Lv2Host::new();
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    let path = b"/dev_flash/sys/external/libnotfound.sprx\0";
    mem.apply_commit(
        cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0x4800), path.len() as u64)
            .unwrap(),
        path,
    )
    .unwrap();
    let rt = FakeRuntime::with_memory(mem);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 480,
            args: [0x4800, 0, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_ENOENT.into())
    );
    assert_eq!(host.observability().prx_load_hle_stub_count, 0);
    assert_eq!(host.observability().prx_load_not_found_count, 1);
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

/// A CoreOS 484 whose import table wraps `u32` links nothing; the
/// walk that never ran must name itself rather than look like an
/// empty table.
#[test]
fn syscall_484_import_table_wrapping_u32_links_nothing_and_names_the_break() {
    use cellgov_mem::{ByteRange as R, GuestAddr};
    let mut host = Lv2Host::new();
    // Authority id whose `>> 36` marks the process as CoreOS.
    host.set_program_authority_id(0x1070_0005_FF00_0001);
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    let mut opt = vec![0u8; 0x30];
    opt[0..8].copy_from_slice(&0x30u64.to_be_bytes());
    opt[8..16].copy_from_slice(&1u64.to_be_bytes());
    opt[0x20..0x24].copy_from_slice(&0xFFFF_F000u32.to_be_bytes());
    opt[0x24..0x28].copy_from_slice(&0x2000u32.to_be_bytes());
    mem.apply_commit(R::new(GuestAddr::new(0x2000), 0x30).unwrap(), &opt)
        .unwrap();
    let rt = FakeRuntime::with_memory(mem);

    let breaks_before = host.observability().invariant_break_count;
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 484,
            args: [0, 0x2000, 0, 0, 0, 0, 0, 0],
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
    assert_eq!(
        host.observability().invariant_break_count - breaks_before,
        1,
        "the refused walk must be witnessed",
    );
}

#[test]
fn syscall_486_with_a_mapped_library_descriptor_returns_ok() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 486,
            args: [0x40, 0, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(result, Lv2Dispatch::immediate(0));
}

#[test]
fn syscall_486_null_library_is_efault_not_a_fabricated_ok() {
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
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
}

#[test]
fn syscall_486_unmapped_library_is_efault() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 486,
            args: [0x8000_0000, 0, 0, 0, 0, 0, 0, 0],
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
fn syscall_484_null_option_pointer_is_einval() {
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
        Lv2Dispatch::immediate(cellgov_ps3_abi::cell_errors::CELL_EINVAL.into())
    );
}

#[cfg(test)]
#[path = "register_module_tests.rs"]
mod register_module;

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

/// Register one module and return `(host, its kernel id)`.
fn host_with_one_prx() -> (Lv2Host, u32) {
    let mut host = Lv2Host::new();
    let id = host.prx_registry_mut().register(
        "libaudio".into(),
        "cellAudio_Library".into(),
        0x0147_0000,
        0x0148_0000,
        0x0147_1000,
        None,
        None,
    );
    (host, id)
}

/// Build a `sys_prx_start_stop_module_option_t` image.
///
/// `size`, `cmd`, and `res` are all `be_t<u64>`, so a `u32` written at
/// offset 0 lands in the HIGH half and reads back as zero.
fn start_stop_option(size: u64, cmd: u64, res: u64) -> [u8; 0x28] {
    let mut buf = [0u8; 0x28];
    buf[0x00..0x08].copy_from_slice(&size.to_be_bytes());
    buf[0x08..0x10].copy_from_slice(&cmd.to_be_bytes());
    buf[0x18..0x20].copy_from_slice(&res.to_be_bytes());
    buf
}

fn runtime_with(p_opt: u32, image: &[u8]) -> FakeRuntime {
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    mem.apply_commit(
        ByteRange::new(
            cellgov_mem::GuestAddr::new(u64::from(p_opt)),
            image.len() as u64,
        )
        .unwrap(),
        image,
    )
    .unwrap();
    FakeRuntime::with_memory(mem)
}

fn start_module(host: &mut Lv2Host, id: u32, p_opt: u32, rt: &FakeRuntime) -> Lv2Dispatch {
    let mut args = [0u64; 8];
    args[0] = u64::from(id);
    args[2] = u64::from(p_opt);
    host.dispatch(
        Lv2Request::Unsupported { number: 481, args },
        UnitId::new(0),
        rt,
    )
}

#[test]
fn prx_start_module_cmd1_writes_no_entry_sentinel() {
    let (mut host, id) = host_with_one_prx();
    let p_opt: u32 = 0x4000;
    let rt = runtime_with(p_opt, &start_stop_option(0x20, 1, 0));
    let result = start_module(&mut host, id, p_opt, &rt);
    let effects = match result {
        Lv2Dispatch::Immediate { code: 0, effects } => effects,
        other => panic!("expected Immediate{{code:0}}, got {other:?}"),
    };
    assert_eq!(
        effects.len(),
        1,
        "size 0x20 has no entry2 field, so only entry is written"
    );
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), u64::from(p_opt + 0x10));
            assert_eq!(range.length(), 8);
            assert_eq!(bytes.bytes(), &u64::MAX.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

/// The size field is a `be_t<u64>`: its low 4 bytes at offset 0 are the
/// high half, zero for every realistic size.
#[test]
fn prx_start_module_reads_size_as_a_full_be_u64() {
    let (mut host, id) = host_with_one_prx();
    let p_opt: u32 = 0x4000;
    let image = start_stop_option(0x20, 1, 0);
    assert_eq!(
        &image[0x00..0x04],
        &[0, 0, 0, 0],
        "a BE u64 size of 0x20 has a zero high half -- the trap"
    );
    let rt = runtime_with(p_opt, &image);
    assert!(
        matches!(
            start_module(&mut host, id, p_opt, &rt),
            Lv2Dispatch::Immediate { code: 0, .. }
        ),
        "size 0x20 is legal; a 4-byte read of the high half would reject it"
    );
}

#[test]
fn prx_start_module_extended_size_also_writes_entry2() {
    let (mut host, id) = host_with_one_prx();
    let p_opt: u32 = 0x4000;
    let rt = runtime_with(p_opt, &start_stop_option(0x28, 1, 0));
    let effects = match start_module(&mut host, id, p_opt, &rt) {
        Lv2Dispatch::Immediate { code: 0, effects } => effects,
        other => panic!("expected Immediate{{code:0}}, got {other:?}"),
    };
    assert_eq!(effects.len(), 2, "size != 0x20 carries entry2");
    let addrs: Vec<u64> = effects
        .iter()
        .map(|e| match e {
            Effect::SharedWriteIntent { range, .. } => range.start().raw(),
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        })
        .collect();
    assert_eq!(
        addrs,
        vec![u64::from(p_opt + 0x10), u64::from(p_opt + 0x20)]
    );
}

#[test]
fn prx_start_module_cmd2_resident_returns_ok_with_no_writes() {
    let (mut host, id) = host_with_one_prx();
    let p_opt: u32 = 0x4000;
    let rt = runtime_with(p_opt, &start_stop_option(0x20, 2, 0));
    assert_eq!(
        start_module(&mut host, id, p_opt, &rt),
        Lv2Dispatch::immediate(0)
    );
}

#[test]
fn prx_start_module_cmd2_non_resident_echoes_res_and_logs_break() {
    let (mut host, id) = host_with_one_prx();
    let p_opt: u32 = 0x4000;
    let rt = runtime_with(p_opt, &start_stop_option(0x20, 2, 0x8001_0002));
    let before = host.observability().invariant_break_count;
    assert_eq!(
        start_module(&mut host, id, p_opt, &rt),
        Lv2Dispatch::immediate(0x8001_0002)
    );
    assert_eq!(host.observability().invariant_break_count - before, 1);
}

#[test]
fn prx_start_module_unknown_cmd_returns_prx_error_and_logs_break() {
    // RPCS3's default arm answers CELL_PRX_ERROR_ERROR, not an LV2
    // errno -- liblv2's dispatcher branches on the 0x8001_1xxx class.
    use cellgov_ps3_abi::sys_prx::CELL_PRX_ERROR_ERROR;
    let (mut host, id) = host_with_one_prx();
    let p_opt: u32 = 0x4000;
    let rt = runtime_with(p_opt, &start_stop_option(0x20, 7, 0));
    let before = host.observability().invariant_break_count;
    assert_eq!(
        start_module(&mut host, id, p_opt, &rt),
        Lv2Dispatch::immediate(CELL_PRX_ERROR_ERROR.into())
    );
    assert_eq!(host.observability().invariant_break_count - before, 1);
}

#[test]
fn prx_start_module_unknown_id_returns_esrch() {
    let (mut host, _id) = host_with_one_prx();
    let p_opt: u32 = 0x4000;
    let rt = runtime_with(p_opt, &start_stop_option(0x20, 1, 0));
    assert_eq!(
        start_module(&mut host, 0xDEAD_BEEF, p_opt, &rt),
        Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into())
    );
}

fn unload_module(host: &mut Lv2Host, id: u32, rt: &FakeRuntime) -> Lv2Dispatch {
    let mut args = [0u64; 8];
    args[0] = u64::from(id);
    host.dispatch(
        Lv2Request::Unsupported { number: 483, args },
        UnitId::new(0),
        rt,
    )
}

#[test]
fn prx_unload_module_started_returns_not_removable_and_counts() {
    use cellgov_ps3_abi::sys_prx::CELL_PRX_ERROR_NOT_REMOVABLE;
    let (mut host, id) = host_with_one_prx();
    host.prx_registry_mut().mark_started(id);
    let rt = FakeRuntime::new(0x1000);
    let breaks_before = host.observability().invariant_break_count;
    assert_eq!(
        unload_module(&mut host, id, &rt),
        Lv2Dispatch::immediate(CELL_PRX_ERROR_NOT_REMOVABLE.into())
    );
    assert_eq!(host.observability().prx_unload_rejections, 1);
    assert_eq!(
        host.observability().invariant_break_count - breaks_before,
        1
    );
}

/// LV2 withdraws an INITIALIZED (never-started) module: a stub the
/// guest loaded and abandoned unloads with CELL_OK, and the id is
/// freed for a later lookup to miss.
#[test]
fn prx_unload_module_unstarted_withdraws_with_ok() {
    use cellgov_ps3_abi::sys_prx::CELL_PRX_ERROR_UNKNOWN_MODULE;
    let (mut host, id) = host_with_one_prx();
    let rt = FakeRuntime::new(0x1000);
    assert_eq!(unload_module(&mut host, id, &rt), Lv2Dispatch::immediate(0));
    assert_eq!(
        host.observability().prx_unload_rejections,
        0,
        "a successful withdraw is not a rejection"
    );
    assert_eq!(
        unload_module(&mut host, id, &rt),
        Lv2Dispatch::immediate(CELL_PRX_ERROR_UNKNOWN_MODULE.into()),
        "the withdrawn id must be gone"
    );
}

/// Completing the sc 481 handshake (cmd=2, res=SYS_PRX_RESIDENT)
/// moves the module to STARTED, which unload then refuses.
#[test]
fn prx_start_handshake_marks_started_and_blocks_unload() {
    use cellgov_ps3_abi::sys_prx::CELL_PRX_ERROR_NOT_REMOVABLE;
    let (mut host, id) = host_with_one_prx();
    let p_opt: u32 = 0x4000;
    let rt = runtime_with(p_opt, &start_stop_option(0x20, 2, 0));
    assert_eq!(
        start_module(&mut host, id, p_opt, &rt),
        Lv2Dispatch::immediate(0)
    );
    assert_eq!(
        unload_module(&mut host, id, &rt),
        Lv2Dispatch::immediate(CELL_PRX_ERROR_NOT_REMOVABLE.into())
    );
}

#[test]
fn prx_unload_module_unknown_id_returns_unknown_module() {
    use cellgov_ps3_abi::sys_prx::CELL_PRX_ERROR_UNKNOWN_MODULE;
    let (mut host, _id) = host_with_one_prx();
    let rt = FakeRuntime::new(0x1000);
    assert_eq!(
        unload_module(&mut host, 0xDEAD_BEEF, &rt),
        Lv2Dispatch::immediate(CELL_PRX_ERROR_UNKNOWN_MODULE.into())
    );
    assert_eq!(
        host.observability().prx_unload_rejections,
        0,
        "the rejection witness counts refusals to unload a real module, not unknown ids"
    );
}

#[test]
fn prx_unload_rejection_witness_starts_at_zero() {
    let (host, _id) = host_with_one_prx();
    assert_eq!(host.observability().prx_unload_rejections, 0);
}

/// RPCS3 never validates `pOpt->size`; it only compares `size != 0x20`
/// to decide whether `entry2` exists. A sub-0x20 size therefore
/// proceeds (and, matching the oracle's comparison, counts as an
/// extended struct writing both entry sentinels).
#[test]
fn syscall_481_accepts_size_below_0x20_like_the_oracle() {
    let (mut host, id) = host_with_one_prx();
    let p_opt: u32 = 0x4000;
    let rt = runtime_with(p_opt, &start_stop_option(0x1F, 1, 0));
    match start_module(&mut host, id, p_opt, &rt) {
        Lv2Dispatch::Immediate { code: 0, effects } => assert_eq!(effects.len(), 2),
        other => panic!("expected Immediate{{code:0}} with two sentinel writes, got {other:?}"),
    }
}

#[test]
fn syscall_481_unreadable_p_opt_returns_efault_and_logs_break() {
    let (mut host, id) = host_with_one_prx();
    let rt = FakeRuntime::new(0x1000);
    let breaks_before = host.observability().invariant_break_count;
    let result = start_module(&mut host, id, 0x4000_1000, &rt);
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
    assert_eq!(
        host.observability().invariant_break_count - breaks_before,
        1
    );
}

#[test]
fn prx_load_module_unreadable_path_pointer_returns_efault() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let path_ptr: u64 = 0x0146_2d58; // far outside the 256-byte memory
    let mut args = [0u64; 8];
    args[0] = path_ptr;
    let result = host.dispatch(
        Lv2Request::Unsupported { number: 480, args },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cellgov_ps3_abi::cell_errors::CELL_EFAULT.into())
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

// -- _sys_prx_stop_module (482) --

fn stop_module(host: &mut Lv2Host, id: u32, p_opt: u32, rt: &FakeRuntime) -> Lv2Dispatch {
    let mut args = [0u64; 8];
    args[0] = u64::from(id);
    args[2] = u64::from(p_opt);
    host.dispatch(
        Lv2Request::Unsupported { number: 482, args },
        UnitId::new(0),
        rt,
    )
}

/// Register one started module and return `(host, its kernel id)`.
fn host_with_one_started_prx() -> (Lv2Host, u32) {
    let (mut host, id) = host_with_one_prx();
    host.prx_registry_mut().mark_started(id);
    (host, id)
}

/// RPCS3's 482 looks the id up before the null-pOpt gate -- the
/// reverse of 481's EINVAL-first order.
#[test]
fn syscall_482_esrch_precedes_einval() {
    let (mut host, id) = host_with_one_started_prx();
    let rt = FakeRuntime::new(0x1000);
    assert_eq!(
        stop_module(&mut host, 0xDEAD_BEEF, 0, &rt),
        Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into()),
        "unknown id with null pOpt reports the id miss, not EINVAL"
    );
    assert_eq!(
        stop_module(&mut host, id, 0, &rt),
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
    );
}

#[test]
fn syscall_482_cmd1_writes_no_entry_and_moves_to_stopping() {
    let (mut host, id) = host_with_one_started_prx();
    let p_opt: u32 = 0x4000;
    let rt = runtime_with(p_opt, &start_stop_option(0x20, 1, 0));
    let effects = match stop_module(&mut host, id, p_opt, &rt) {
        Lv2Dispatch::Immediate { code: 0, effects } => effects,
        other => panic!("expected Immediate{{code:0}}, got {other:?}"),
    };
    assert_eq!(
        effects.len(),
        1,
        "size 0x20 has no entry2 field, so only entry is written"
    );
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), u64::from(p_opt + 0x10));
            assert_eq!(range.length(), 8);
            assert_eq!(bytes.bytes(), &u64::MAX.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
    use crate::prx_registry::PrxState;
    assert_eq!(
        host.prx_registry().lookup_by_id(id).unwrap().state(),
        PrxState::Stopping
    );
}

#[test]
fn syscall_482_cmd1_extended_size_also_writes_entry2() {
    let (mut host, id) = host_with_one_started_prx();
    let p_opt: u32 = 0x4000;
    let rt = runtime_with(p_opt, &start_stop_option(0x28, 1, 0));
    let effects = match stop_module(&mut host, id, p_opt, &rt) {
        Lv2Dispatch::Immediate { code: 0, effects } => effects,
        other => panic!("expected Immediate{{code:0}}, got {other:?}"),
    };
    assert_eq!(effects.len(), 2, "size != 0x20 carries entry2");
}

#[test]
fn syscall_482_cmd1_wrong_states_report_their_codes() {
    use cellgov_ps3_abi::sys_prx::{
        CELL_PRX_ERROR_ALREADY_STOPPED, CELL_PRX_ERROR_ALREADY_STOPPING, CELL_PRX_ERROR_NOT_STARTED,
    };
    let p_opt: u32 = 0x4000;
    let image = start_stop_option(0x20, 1, 0);

    let (mut host, id) = host_with_one_prx();
    let rt = runtime_with(p_opt, &image);
    assert_eq!(
        stop_module(&mut host, id, p_opt, &rt),
        Lv2Dispatch::immediate(CELL_PRX_ERROR_NOT_STARTED.into()),
        "Initialized reports NOT_STARTED"
    );

    let (mut host, id) = host_with_one_started_prx();
    let rt = runtime_with(p_opt, &image);
    assert!(matches!(
        stop_module(&mut host, id, p_opt, &rt),
        Lv2Dispatch::Immediate { code: 0, .. }
    ));
    assert_eq!(
        stop_module(&mut host, id, p_opt, &rt),
        Lv2Dispatch::immediate(CELL_PRX_ERROR_ALREADY_STOPPING.into()),
        "a second phase 1 reports ALREADY_STOPPING"
    );
    host.prx_registry_mut().finish_stop(id);
    assert_eq!(
        stop_module(&mut host, id, p_opt, &rt),
        Lv2Dispatch::immediate(CELL_PRX_ERROR_ALREADY_STOPPED.into()),
        "a stop after completion reports ALREADY_STOPPED"
    );
}

/// Phase 1 hands back NO_ENTRY, so the guest calls nothing between
/// the two phases.
#[test]
fn syscall_482_full_handshake_unblocks_unload() {
    use cellgov_ps3_abi::sys_prx::CELL_PRX_ERROR_UNKNOWN_MODULE;
    let (mut host, id) = host_with_one_started_prx();
    let p_opt: u32 = 0x4000;
    let rt = runtime_with(p_opt, &start_stop_option(0x20, 1, 0));
    assert!(matches!(
        stop_module(&mut host, id, p_opt, &rt),
        Lv2Dispatch::Immediate { code: 0, .. }
    ));
    let rt = runtime_with(p_opt, &start_stop_option(0x20, 2, 0));
    let breaks_before = host.observability().invariant_break_count;
    assert_eq!(
        stop_module(&mut host, id, p_opt, &rt),
        Lv2Dispatch::immediate(0)
    );
    assert_eq!(
        host.observability().invariant_break_count,
        breaks_before,
        "a well-ordered handshake is not an invariant break"
    );
    assert_eq!(unload_module(&mut host, id, &rt), Lv2Dispatch::immediate(0));
    assert_eq!(
        unload_module(&mut host, id, &rt),
        Lv2Dispatch::immediate(CELL_PRX_ERROR_UNKNOWN_MODULE.into()),
        "the withdrawn id must be gone"
    );
}

#[test]
fn syscall_482_stopping_module_still_refuses_unload() {
    use cellgov_ps3_abi::sys_prx::CELL_PRX_ERROR_NOT_REMOVABLE;
    let (mut host, id) = host_with_one_started_prx();
    let p_opt: u32 = 0x4000;
    let rt = runtime_with(p_opt, &start_stop_option(0x20, 1, 0));
    assert!(matches!(
        stop_module(&mut host, id, p_opt, &rt),
        Lv2Dispatch::Immediate { code: 0, .. }
    ));
    assert_eq!(
        unload_module(&mut host, id, &rt),
        Lv2Dispatch::immediate(CELL_PRX_ERROR_NOT_REMOVABLE.into())
    );
}

/// cmd=2 res=0 without an accepted phase 1: RPCS3 hard-asserts, so
/// there is no oracle behaviour; CellGov logs the break, returns
/// CELL_OK, and leaves the state alone.
#[test]
fn syscall_482_cmd2_without_phase1_logs_break_and_keeps_state() {
    use crate::prx_registry::PrxState;
    let (mut host, id) = host_with_one_started_prx();
    let p_opt: u32 = 0x4000;
    let rt = runtime_with(p_opt, &start_stop_option(0x20, 2, 0));
    let breaks_before = host.observability().invariant_break_count;
    assert_eq!(
        stop_module(&mut host, id, p_opt, &rt),
        Lv2Dispatch::immediate(0)
    );
    assert_eq!(
        host.observability().invariant_break_count - breaks_before,
        1
    );
    assert_eq!(
        host.prx_registry().lookup_by_id(id).unwrap().state(),
        PrxState::Started
    );
}

#[test]
fn syscall_482_cmd2_res1_returns_can_not_stop_and_logs_break() {
    use cellgov_ps3_abi::sys_prx::CELL_PRX_ERROR_CAN_NOT_STOP;
    let (mut host, id) = host_with_one_started_prx();
    let p_opt: u32 = 0x4000;
    let rt = runtime_with(p_opt, &start_stop_option(0x20, 2, 1));
    let breaks_before = host.observability().invariant_break_count;
    assert_eq!(
        stop_module(&mut host, id, p_opt, &rt),
        Lv2Dispatch::immediate(CELL_PRX_ERROR_CAN_NOT_STOP.into())
    );
    assert_eq!(
        host.observability().invariant_break_count - breaks_before,
        1
    );
}

#[test]
fn syscall_482_cmd2_other_res_returns_ok_without_transition() {
    use crate::prx_registry::PrxState;
    let (mut host, id) = host_with_one_started_prx();
    let p_opt: u32 = 0x4000;
    // Move to Stopping first so a buggy arm that transitions anyway
    // would be caught by the state assert.
    let rt = runtime_with(p_opt, &start_stop_option(0x20, 1, 0));
    stop_module(&mut host, id, p_opt, &rt);
    let rt = runtime_with(p_opt, &start_stop_option(0x20, 2, 0x8001_0002));
    let breaks_before = host.observability().invariant_break_count;
    assert_eq!(
        stop_module(&mut host, id, p_opt, &rt),
        Lv2Dispatch::immediate(0),
        "RPCS3's default res arm returns CELL_OK"
    );
    assert_eq!(
        host.observability().invariant_break_count - breaks_before,
        1
    );
    assert_eq!(
        host.prx_registry().lookup_by_id(id).unwrap().state(),
        PrxState::Stopping,
        "only res=0 completes the stop"
    );
}

/// Repeating cmd=4 still succeeds, since it never transitions.
#[test]
fn syscall_482_cmd4_writes_entries_and_keeps_started() {
    use crate::prx_registry::PrxState;
    let (mut host, id) = host_with_one_started_prx();
    let p_opt: u32 = 0x4000;
    let rt = runtime_with(p_opt, &start_stop_option(0x28, 4, 0));
    let effects = match stop_module(&mut host, id, p_opt, &rt) {
        Lv2Dispatch::Immediate { code: 0, effects } => effects,
        other => panic!("expected Immediate{{code:0}}, got {other:?}"),
    };
    assert_eq!(effects.len(), 2, "extended struct writes entry and entry2");
    assert_eq!(
        host.prx_registry().lookup_by_id(id).unwrap().state(),
        PrxState::Started
    );
    assert!(matches!(
        stop_module(&mut host, id, p_opt, &rt),
        Lv2Dispatch::Immediate { code: 0, .. }
    ));
}

/// cmd=8 (and any nibble-4/8 value that is not exactly 4, e.g. 0x14)
/// takes RPCS3's todo arm: CELL_OK, no writes, no state change.
#[test]
fn syscall_482_cmd8_is_a_no_op_stub_that_logs_break() {
    use crate::prx_registry::PrxState;
    for cmd in [8u64, 0x14, 0x18] {
        let (mut host, id) = host_with_one_started_prx();
        let p_opt: u32 = 0x4000;
        let rt = runtime_with(p_opt, &start_stop_option(0x20, cmd, 0));
        let breaks_before = host.observability().invariant_break_count;
        assert_eq!(
            stop_module(&mut host, id, p_opt, &rt),
            Lv2Dispatch::immediate(0),
            "cmd={cmd:#x}"
        );
        assert_eq!(
            host.observability().invariant_break_count - breaks_before,
            1
        );
        assert_eq!(
            host.prx_registry().lookup_by_id(id).unwrap().state(),
            PrxState::Started
        );
    }
}

#[test]
fn syscall_482_cmd4_wrong_state_reports_not_started() {
    use cellgov_ps3_abi::sys_prx::CELL_PRX_ERROR_NOT_STARTED;
    let (mut host, id) = host_with_one_prx();
    let p_opt: u32 = 0x4000;
    let rt = runtime_with(p_opt, &start_stop_option(0x20, 4, 0));
    assert_eq!(
        stop_module(&mut host, id, p_opt, &rt),
        Lv2Dispatch::immediate(CELL_PRX_ERROR_NOT_STARTED.into())
    );
}

#[test]
fn syscall_482_unknown_cmd_returns_prx_error_and_logs_break() {
    use cellgov_ps3_abi::sys_prx::CELL_PRX_ERROR_ERROR;
    let (mut host, id) = host_with_one_started_prx();
    let p_opt: u32 = 0x4000;
    let rt = runtime_with(p_opt, &start_stop_option(0x20, 7, 0));
    let before = host.observability().invariant_break_count;
    assert_eq!(
        stop_module(&mut host, id, p_opt, &rt),
        Lv2Dispatch::immediate(CELL_PRX_ERROR_ERROR.into())
    );
    assert_eq!(host.observability().invariant_break_count - before, 1);
}

#[test]
fn syscall_482_unreadable_p_opt_returns_efault_and_logs_break() {
    let (mut host, id) = host_with_one_started_prx();
    let rt = FakeRuntime::new(0x1000);
    let breaks_before = host.observability().invariant_break_count;
    assert_eq!(
        stop_module(&mut host, id, 0x4000_1000, &rt),
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
    assert_eq!(
        host.observability().invariant_break_count - breaks_before,
        1
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
    let breaks_before = host.observability().invariant_break_count;
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
    assert_eq!(
        host.observability().invariant_break_count - breaks_before,
        1
    );
}

#[test]
fn syscall_494_unreadable_idlist_field_returns_efault_and_logs_break() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let breaks_before = host.observability().invariant_break_count;
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
    assert_eq!(
        host.observability().invariant_break_count - breaks_before,
        1
    );
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

// A wrapping u32 pointer must return EFAULT rather than emit a
// wrong-address SharedWriteIntent behind a CELL_OK.

#[test]
fn prx_start_module_wrapping_p_opt_returns_efault_and_emits_no_writes() {
    use crate::host::Lv2Runtime;
    use cellgov_time::GuestTicks;
    struct WrapMock {
        size_be: [u8; 8],
    }
    impl Lv2Runtime for WrapMock {
        fn committed_overlap_end(&self, _addr: u64, _size: u64) -> Option<u64> {
            // No region model; every window reads as free.
            None
        }

        fn read_committed(&self, _addr: u64, len: usize) -> Option<&[u8]> {
            (len == 8).then_some(&self.size_be[..])
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
    // A registered id is required: the ESRCH lookup precedes the
    // wrap check, matching RPCS3's order, so an unknown id would
    // never reach the path under test.
    let (mut host, id) = host_with_one_prx();
    let breaks_before = host.observability().invariant_break_count;
    let rt = WrapMock {
        size_be: 0x20u64.to_be_bytes(),
    };
    // entry2 at p_opt+0x28 is the furthest field the struct can
    // reach, so 0xFFFF_FFF0 wraps u32 before the write is staged.
    let mut args = [0u64; 8];
    args[0] = u64::from(id);
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
        host.observability().invariant_break_count - breaks_before,
        1,
        "wrap path must log_invariant_break exactly once"
    );
}

#[test]
fn prx_get_module_list_wrapping_p_info_returns_efault_and_emits_no_writes() {
    use crate::host::Lv2Runtime;
    use cellgov_time::GuestTicks;
    // Returns 4 zero bytes for every read, so without the wrap check
    // the arm would reach the count-write at count_addr = pInfo+0x10,
    // which wraps to addr 0, behind a CELL_OK.
    struct ZeroReadMock {
        zeros: [u8; 4],
    }
    impl Lv2Runtime for ZeroReadMock {
        fn committed_overlap_end(&self, _addr: u64, _size: u64) -> Option<u64> {
            // No region model; every window reads as free.
            None
        }

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
    let breaks_before = host.observability().invariant_break_count;
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
        host.observability().invariant_break_count - breaks_before,
        1,
        "wrap path must log_invariant_break exactly once"
    );
}
