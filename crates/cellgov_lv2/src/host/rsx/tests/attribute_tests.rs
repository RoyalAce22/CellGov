//! `sys_rsx_context_attribute` dispatch tests: flip-request emission, buffer-index resolution, and out-of-range nibble fallback.

use super::*;
use crate::host::rsx::state::RSX_CONTEXT_ID;
use crate::host::rsx::test_helpers::context_allocate_request;
use crate::host::test_support::FakeRuntime;
use crate::request::Lv2Request;
use cellgov_event::UnitId;

fn allocate_context(host: &mut Lv2Host, source: UnitId) {
    let rt = FakeRuntime::new(0x1_0000);
    let d = host.dispatch(
        context_allocate_request(0x1000, 0x1008, 0x1010, 0x1018, 0xA001),
        source,
        &rt,
    );
    assert!(matches!(d, Lv2Dispatch::Immediate { code: 0, .. }));
}

#[test]
fn sys_rsx_context_attribute_flip_emits_flip_request() {
    let mut host = Lv2Host::new();
    let source = UnitId::new(0);
    allocate_context(&mut host, source);

    let rt = FakeRuntime::new(0x1_0000);
    let d = host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: RSX_CONTEXT_ID,
            package_id: package::FLIP_BUFFER,
            a3: 0,
            a4: 0x8000_0003,
            a5: 0,
            a6: 0,
        },
        source,
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = d else {
        panic!("expected Immediate(0), got {d:?}");
    };
    assert_eq!(effects.len(), 1);
    assert!(matches!(
        effects[0],
        Effect::RsxFlipRequest { buffer_index: 3 }
    ));
}

#[test]
fn sys_rsx_context_attribute_flip_queued_path_out_of_range_nibble_falls_back_with_witness() {
    let mut host = Lv2Host::new();
    let source = UnitId::new(0);
    allocate_context(&mut host, source);
    let pre_breaks = host.invariant_break_count();

    let rt = FakeRuntime::new(0x1_0000);
    let d = host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: RSX_CONTEXT_ID,
            package_id: package::FLIP_BUFFER,
            a3: 0,
            a4: 0x8000_0009, // queued path, nibble = 0x9 (out of range)
            a5: 0,
            a6: 0,
        },
        source,
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = d else {
        panic!("expected Immediate(0), got {d:?}");
    };
    assert!(matches!(
        effects[0],
        Effect::RsxFlipRequest { buffer_index: 0 }
    ));
    assert!(
        host.invariant_break_count() > pre_breaks,
        "out-of-range nibble fallback must witness a log_invariant_break \
         so a future consumer can disambiguate slot-0-was-requested from \
         clamped-from-9",
    );
}

#[test]
fn sys_rsx_context_attribute_flip_direct_path_resolves_index_by_offset_match() {
    let mut host = Lv2Host::new();
    let source = UnitId::new(0);
    allocate_context(&mut host, source);

    let rt = FakeRuntime::new(0x1_0000);
    // Register slot 0 with offset 0x10_0000, slot 1 with offset 0x20_0000.
    for (id, off) in [(0u64, 0x10_0000u32), (1u64, 0x20_0000u32)] {
        host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: package::SET_DISPLAY_BUFFER,
                a3: id,
                a4: (1920u64 << 32) | 1080,
                a5: (0x2000u64 << 32) | (off as u64),
                a6: 0,
            },
            source,
            &rt,
        );
    }

    let d = host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: RSX_CONTEXT_ID,
            package_id: package::FLIP_BUFFER,
            a3: 0,
            a4: 0x20_0000, // matches slot 1
            a5: 0,
            a6: 0,
        },
        source,
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = d else {
        panic!("expected Immediate(0), got {d:?}");
    };
    assert!(
        matches!(effects[0], Effect::RsxFlipRequest { buffer_index: 1 }),
        "direct-path FLIP_BUFFER must resolve flip_target=0x20_0000 to slot 1 \
         (whose offset matches); fabricating 0 here would silently lose the \
         buffer identity for any future consumer that reads buffer_index",
    );
}

#[test]
fn sys_rsx_context_attribute_flip_direct_path_no_match_falls_back_to_zero() {
    let mut host = Lv2Host::new();
    let source = UnitId::new(0);
    allocate_context(&mut host, source);
    let pre_breaks = host.invariant_break_count();

    let rt = FakeRuntime::new(0x1_0000);
    let d = host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: RSX_CONTEXT_ID,
            package_id: package::FLIP_BUFFER,
            a3: 0,
            a4: 0x0000_1234, // no display buffer registered with this offset
            a5: 0,
            a6: 0,
        },
        source,
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = d else {
        panic!("expected Immediate(0), got {d:?}");
    };
    assert!(matches!(
        effects[0],
        Effect::RsxFlipRequest { buffer_index: 0 }
    ));
    assert!(
        host.invariant_break_count() > pre_breaks,
        "no-match fallback must witness a log_invariant_break so the \
         silent-substitution is non-vacuous; otherwise the 0 we synthesize \
         would be indistinguishable from a successful match against slot 0",
    );
}

#[test]
fn sys_rsx_context_attribute_set_flip_handler_records_callback() {
    let mut host = Lv2Host::new();
    let source = UnitId::new(0);
    allocate_context(&mut host, source);

    let rt = FakeRuntime::new(0x1_0000);
    let d = host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: RSX_CONTEXT_ID,
            package_id: PACKAGE_CELLGOV_SET_FLIP_HANDLER,
            a3: 0xDEAD_BEEF,
            a4: 0,
            a5: 0,
            a6: 0,
        },
        source,
        &rt,
    );
    assert!(matches!(d, Lv2Dispatch::Immediate { code: 0, .. }));
    assert_eq!(host.sys_rsx_context().flip_handler_addr, 0xDEAD_BEEF);
    assert_eq!(host.sys_rsx_context().vblank_handler_addr, 0);
    assert_eq!(host.sys_rsx_context().user_handler_addr, 0);
}

#[test]
fn sys_rsx_context_attribute_set_vblank_handler_records_callback() {
    let mut host = Lv2Host::new();
    let source = UnitId::new(0);
    allocate_context(&mut host, source);

    let rt = FakeRuntime::new(0x1_0000);
    host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: RSX_CONTEXT_ID,
            package_id: PACKAGE_CELLGOV_SET_VBLANK_HANDLER,
            a3: 0xCAFE_F00D,
            a4: 0,
            a5: 0,
            a6: 0,
        },
        source,
        &rt,
    );
    assert_eq!(host.sys_rsx_context().vblank_handler_addr, 0xCAFE_F00D);
}

#[test]
fn sys_rsx_context_attribute_set_user_handler_records_callback() {
    let mut host = Lv2Host::new();
    let source = UnitId::new(0);
    allocate_context(&mut host, source);

    let rt = FakeRuntime::new(0x1_0000);
    host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: RSX_CONTEXT_ID,
            package_id: PACKAGE_CELLGOV_SET_USER_HANDLER,
            a3: 0xABCD_0001,
            a4: 0,
            a5: 0,
            a6: 0,
        },
        source,
        &rt,
    );
    assert_eq!(host.sys_rsx_context().user_handler_addr, 0xABCD_0001);
}

#[test]
fn sys_rsx_context_attribute_null_flip_handler_clears() {
    let mut host = Lv2Host::new();
    let source = UnitId::new(0);
    allocate_context(&mut host, source);
    let rt = FakeRuntime::new(0x1_0000);
    host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: RSX_CONTEXT_ID,
            package_id: PACKAGE_CELLGOV_SET_FLIP_HANDLER,
            a3: 0x1234_5678,
            a4: 0,
            a5: 0,
            a6: 0,
        },
        source,
        &rt,
    );
    host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: RSX_CONTEXT_ID,
            package_id: PACKAGE_CELLGOV_SET_FLIP_HANDLER,
            a3: 0,
            a4: 0,
            a5: 0,
            a6: 0,
        },
        source,
        &rt,
    );
    assert_eq!(host.sys_rsx_context().flip_handler_addr, 0);
}

#[test]
fn sys_rsx_context_attribute_flip_mode_records_mode() {
    let mut host = Lv2Host::new();
    let source = UnitId::new(0);
    allocate_context(&mut host, source);

    let rt = FakeRuntime::new(0x1_0000);
    host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: RSX_CONTEXT_ID,
            package_id: package::FLIP_MODE,
            a3: 0,
            a4: 2,
            a5: 0,
            a6: 0,
        },
        source,
        &rt,
    );
    assert_eq!(host.sys_rsx_context().flip_mode, 2);
}

#[test]
fn sys_rsx_context_attribute_set_display_buffer_records_slot() {
    let mut host = Lv2Host::new();
    let source = UnitId::new(0);
    allocate_context(&mut host, source);

    let rt = FakeRuntime::new(0x1_0000);
    host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: RSX_CONTEXT_ID,
            package_id: package::SET_DISPLAY_BUFFER,
            a3: 1,
            a4: (1920u64 << 32) | 1080,
            a5: (0x2000u64 << 32) | 0x10_0000,
            a6: 0,
        },
        source,
        &rt,
    );
    let ctx = host.sys_rsx_context();
    assert_eq!(ctx.display_buffers_count, 2);
    let slot = ctx.display_buffers[1];
    assert_eq!(slot.width, 1920);
    assert_eq!(slot.height, 1080);
    assert_eq!(slot.pitch, 0x2000);
    assert_eq!(slot.offset, 0x10_0000);
}

#[test]
fn sys_rsx_context_attribute_set_display_buffer_sparse_registration_leaves_count_dense() {
    // Spec: `display_buffers_count = max(id + 1, count)`. The
    // state hash captures the full slot array including
    // uninitialized entries; a future compaction would break
    // determinism.
    let mut host = Lv2Host::new();
    let source = UnitId::new(0);
    allocate_context(&mut host, source);

    let rt = FakeRuntime::new(0x1_0000);
    host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: RSX_CONTEXT_ID,
            package_id: package::SET_DISPLAY_BUFFER,
            a3: 3,
            a4: (1920u64 << 32) | 1080,
            a5: (0x2000u64 << 32) | 0xAABB_CCDD,
            a6: 0,
        },
        source,
        &rt,
    );
    let ctx = host.sys_rsx_context();
    assert_eq!(
        ctx.display_buffers_count, 4,
        "sparse registration of id=3 must leave count=4 (max(3+1, 0))",
    );
    assert_eq!(ctx.display_buffers[3].offset, 0xAABB_CCDD);
    // Slots 0..3 were never written; they hold their init-fill
    // value (zero). The hash includes these.
    for slot in &ctx.display_buffers[..3] {
        assert_eq!(slot.offset, 0);
        assert_eq!(slot.pitch, 0);
        assert_eq!(slot.width, 0);
        assert_eq!(slot.height, 0);
    }
}

#[test]
fn sys_rsx_context_attribute_set_display_buffer_rejects_id_over_7() {
    let mut host = Lv2Host::new();
    let source = UnitId::new(0);
    allocate_context(&mut host, source);

    let rt = FakeRuntime::new(0x1_0000);
    let d = host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: RSX_CONTEXT_ID,
            package_id: package::SET_DISPLAY_BUFFER,
            a3: 8,
            a4: 0,
            a5: 0,
            a6: 0,
        },
        source,
        &rt,
    );
    assert!(matches!(
        d,
        Lv2Dispatch::Immediate { code, .. } if code == u64::from(cell_errors::CELL_EINVAL)
    ));
}

#[test]
fn sys_rsx_context_attribute_unknown_package_returns_einval() {
    let mut host = Lv2Host::new();
    let source = UnitId::new(0);
    allocate_context(&mut host, source);

    let rt = FakeRuntime::new(0x1_0000);
    let d = host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: RSX_CONTEXT_ID,
            package_id: 0xBEEF,
            a3: 0,
            a4: 0,
            a5: 0,
            a6: 0,
        },
        source,
        &rt,
    );
    let expected = u64::from(cell_errors::CELL_EINVAL);
    assert!(matches!(
        d,
        Lv2Dispatch::Immediate { code, effects } if code == expected && effects.is_empty()
    ));
}

#[test]
fn sys_rsx_context_attribute_rejects_unallocated_context() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1_0000);
    let d = host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: RSX_CONTEXT_ID,
            package_id: 0x102,
            a3: 0,
            a4: 0,
            a5: 0,
            a6: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert!(matches!(
        d,
        Lv2Dispatch::Immediate { code, .. } if code == u64::from(cell_errors::CELL_EINVAL)
    ));
}

#[test]
fn sys_rsx_context_attribute_fifo_setup_records_get_and_put() {
    let mut host = Lv2Host::new();
    let source = UnitId::new(0);
    allocate_context(&mut host, source);

    // Default FakeRuntime memory is 64 KiB, so MMIO at
    // 0xC000_0040 / 0xC000_0044 reads as non-writable -- the
    // reserved-region title path. FIFO_SETUP records the
    // pointers and emits ZERO effects.
    let rt = FakeRuntime::new(0x1_0000);
    let d = host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: RSX_CONTEXT_ID,
            package_id: package::FIFO_SETUP,
            a3: 0x1000,
            a4: 0x2000,
            a5: 0,
            a6: 0,
        },
        source,
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = d else {
        panic!("expected Immediate(0), got {d:?}");
    };
    assert!(
        effects.is_empty(),
        "reserved-region (MMIO non-writable) FIFO_SETUP must emit no \
         SharedWriteIntent effects; otherwise the next batch's commit \
         would fail validation",
    );
    let ctx = host.sys_rsx_context();
    assert_eq!(ctx.fifo_get, 0x1000);
    assert_eq!(ctx.fifo_put, 0x2000);
}

#[test]
fn sys_rsx_context_attribute_fifo_setup_emits_mmio_writes_when_writable() {
    // 40F honest-consumer companion: when the MMIO control-
    // register slots ARE writable (rsx_mirror=true title that
    // re-maps the RSX region writable), FIFO_SETUP emits two
    // SharedWriteIntent effects -- a4 (put) -> 0xC000_0040,
    // a3 (get) -> 0xC000_0044 -- so the engine-side cursor
    // sees the initial pointers from syscall return without
    // waiting for the title's first store. The effect
    // ordering (put first, then get) matches the cursor->MMIO
    // writeback in commit_step::mirror_rsx_cursor_to_mmio.
    let mut host = Lv2Host::new();
    let source = UnitId::new(0);
    allocate_context(&mut host, source);

    let rt = FakeRuntime::new(0x1_0000).with_writable_override(true);
    let d = host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: RSX_CONTEXT_ID,
            package_id: package::FIFO_SETUP,
            a3: 0x1100,
            a4: 0x2200,
            a5: 0,
            a6: 0,
        },
        source,
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = d else {
        panic!("expected Immediate(0), got {d:?}");
    };
    assert_eq!(
        effects.len(),
        2,
        "writable MMIO path must emit exactly the put and get writebacks",
    );

    // Effect 0: put -> PUT_ADDR (0xC000_0040), value = a4.
    let Effect::SharedWriteIntent {
        range: range0,
        bytes: bytes0,
        ..
    } = &effects[0]
    else {
        panic!(
            "expected SharedWriteIntent at index 0, got {:?}",
            effects[0]
        );
    };
    assert_eq!(
        range0.start().raw(),
        control_register::PUT_ADDR as u64,
        "effect 0 must target PUT_ADDR, not GET_ADDR (the cursor->MMIO \
         writeback in commit_step uses the same put-then-get ordering)",
    );
    assert_eq!(range0.length(), 4);
    assert_eq!(
        u32::from_be_bytes(bytes0.bytes().try_into().unwrap()),
        0x2200,
        "PUT slot must carry a4 (the put pointer), not a3",
    );

    // Effect 1: get -> GET_ADDR (0xC000_0044), value = a3.
    let Effect::SharedWriteIntent {
        range: range1,
        bytes: bytes1,
        ..
    } = &effects[1]
    else {
        panic!(
            "expected SharedWriteIntent at index 1, got {:?}",
            effects[1]
        );
    };
    assert_eq!(
        range1.start().raw(),
        control_register::GET_ADDR as u64,
        "effect 1 must target GET_ADDR; the put/get ordering swap would \
         type-check cleanly so this assertion is the catch",
    );
    assert_eq!(range1.length(), 4);
    assert_eq!(
        u32::from_be_bytes(bytes1.bytes().try_into().unwrap()),
        0x1100,
        "GET slot must carry a3 (the get pointer), not a4",
    );

    let ctx = host.sys_rsx_context();
    assert_eq!(ctx.fifo_get, 0x1100);
    assert_eq!(ctx.fifo_put, 0x2200);
}

/// F5 witness: the FIFO_SETUP writability gate is the conjunction
/// `put_writable && get_writable`. Today's tests cover both
/// writable (emit) and both non-writable (skip), but a refactor
/// that drops the get-side check would pass both. These two
/// asymmetric-writability tests prove each side independently
/// gates the emit.
#[test]
fn sys_rsx_context_attribute_fifo_setup_skips_emit_when_only_put_writable() {
    let mut host = Lv2Host::new();
    let source = UnitId::new(0);
    allocate_context(&mut host, source);

    let rt = FakeRuntime::new(0x1_0000)
        .with_writable_at(control_register::PUT_ADDR as u64, true)
        .with_writable_at(control_register::GET_ADDR as u64, false);
    let d = host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: RSX_CONTEXT_ID,
            package_id: package::FIFO_SETUP,
            a3: 0x1100,
            a4: 0x2200,
            a5: 0,
            a6: 0,
        },
        source,
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = d else {
        panic!("expected Immediate(0), got {d:?}");
    };
    assert!(
        effects.is_empty(),
        "get-side non-writable must gate the emit even with put writable; \
         dropping the get_writable check from the conjunction would let \
         this case emit and the next batch's commit would fail validation",
    );
}

#[test]
fn sys_rsx_context_attribute_fifo_setup_skips_emit_when_only_get_writable() {
    let mut host = Lv2Host::new();
    let source = UnitId::new(0);
    allocate_context(&mut host, source);

    let rt = FakeRuntime::new(0x1_0000)
        .with_writable_at(control_register::PUT_ADDR as u64, false)
        .with_writable_at(control_register::GET_ADDR as u64, true);
    let d = host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: RSX_CONTEXT_ID,
            package_id: package::FIFO_SETUP,
            a3: 0x1100,
            a4: 0x2200,
            a5: 0,
            a6: 0,
        },
        source,
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = d else {
        panic!("expected Immediate(0), got {d:?}");
    };
    assert!(
        effects.is_empty(),
        "put-side non-writable must gate the emit even with get writable; \
         symmetric to the only-put-writable case, locks the conjunction \
         from either direction",
    );
}

#[test]
fn sys_rsx_context_attribute_after_free_still_dispatches() {
    // Pins the noop-free contract: allocate -> free ->
    // context_attribute(same id) succeeds. If a future multi-
    // context model lands, free must clear `allocated` and this
    // test gets inverted.
    let mut host = Lv2Host::new();
    let source = UnitId::new(0);
    allocate_context(&mut host, source);

    let rt = FakeRuntime::new(0x1_0000);
    host.dispatch(
        Lv2Request::SysRsxContextFree {
            context_id: RSX_CONTEXT_ID,
        },
        source,
        &rt,
    );

    let d = host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: RSX_CONTEXT_ID,
            package_id: PACKAGE_CELLGOV_SET_FLIP_HANDLER,
            a3: 0x1234_5678,
            a4: 0,
            a5: 0,
            a6: 0,
        },
        source,
        &rt,
    );
    assert!(
        matches!(d, Lv2Dispatch::Immediate { code: 0, .. }),
        "context_attribute after a noop-free must still dispatch \
         cleanly in the single-context model; allocated is never cleared",
    );
    assert_eq!(host.sys_rsx_context().flip_handler_addr, 0x1234_5678);
}

#[test]
fn sys_rsx_context_attribute_rejects_wrong_context_id() {
    let mut host = Lv2Host::new();
    let source = UnitId::new(0);
    allocate_context(&mut host, source);

    let rt = FakeRuntime::new(0x1_0000);
    let d = host.dispatch(
        Lv2Request::SysRsxContextAttribute {
            context_id: 0xDEAD_BEEF,
            package_id: 0x102,
            a3: 0,
            a4: 0,
            a5: 0,
            a6: 0,
        },
        source,
        &rt,
    );
    assert!(matches!(
        d,
        Lv2Dispatch::Immediate { code, .. } if code == u64::from(cell_errors::CELL_EINVAL)
    ));
}
