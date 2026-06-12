//! NV command-header decoding, method-table registration, and NV406E/NV4097 handler behavior.

use super::*;

fn hdr(cmd: u32) -> NvMethodHeader {
    decode_header(cmd)
}

// Mask both fields so a typo cannot silently set a control-flow flag bit.
fn method_header(method: u32, count: u32) -> u32 {
    ((count & NV_COUNT_MASK_11) << NV_COUNT_SHIFT) | (method & NV_METHOD_MASK)
}

fn method_header_ni(method: u32, count: u32) -> u32 {
    method_header(method, count) | NV_FLAG_NON_INCREMENT
}

#[test]
fn nop_is_zero_count_increment() {
    let h = hdr(0x0000_0000);
    assert_eq!(h, NvMethodHeader::normal(NvCommandKind::Increment, 0, 0));
}

#[test]
fn increment_normal_method() {
    let h = hdr(method_header(NV406E_SET_REFERENCE as u32, 1));
    assert_eq!(h.kind, NvCommandKind::Increment);
    assert_eq!(h.method, NV406E_SET_REFERENCE);
    assert_eq!(h.count, 1);
}

#[test]
fn increment_semaphore_pair() {
    let off = hdr(method_header(NV406E_SEMAPHORE_OFFSET as u32, 1));
    let rel = hdr(method_header(NV406E_SEMAPHORE_RELEASE as u32, 1));
    assert_eq!(off.method, 0x0064);
    assert_eq!(rel.method, 0x006C);
    assert_eq!(off.count, 1);
    assert_eq!(rel.count, 1);
}

#[test]
fn non_increment_method() {
    let h = hdr(method_header_ni(0x1818, 32));
    assert_eq!(h.kind, NvCommandKind::NonIncrement);
    assert_eq!(h.method, 0x1818);
    assert_eq!(h.count, 32);
}

#[test]
fn count_field_spans_11_bits() {
    let h = hdr(method_header(0x0100, 2047));
    assert_eq!(h.count, 2047);
    let h = hdr(method_header(0x0100, 0x7FF));
    assert_eq!(h.count, 0x7FF);
}

#[test]
fn method_address_preserves_subchannel_bits() {
    let h = hdr(method_header(GCM_FLIP_COMMAND as u32, 1));
    assert_eq!(h.method, 0xFEAC);
}

#[test]
fn method_address_zero_in_low_bits() {
    let raw = method_header(0x0053, 1);
    let h = hdr(raw);
    assert_eq!(h.method, 0x0050, "low 2 bits masked off");
}

#[test]
fn old_jump_decodes() {
    let h = hdr(0x2000_1000 | NV_FLAG_JUMP);
    assert_eq!(h.kind, NvCommandKind::Jump { offset: 0x1000 });
    let h = hdr(NV_FLAG_JUMP | 0x0004_0000);
    assert_eq!(
        h.kind,
        NvCommandKind::Jump {
            offset: 0x0004_0000
        }
    );
}

#[test]
fn old_jump_maximum_offset_fits_29_bits() {
    let h = hdr(NV_FLAG_JUMP | 0x1FFF_FFFC);
    assert_eq!(
        h.kind,
        NvCommandKind::Jump {
            offset: 0x1FFF_FFFC
        }
    );
}

#[test]
fn old_jump_with_bit_31_set_is_malformed() {
    let h = hdr(0x8000_0000 | NV_FLAG_JUMP);
    match h.kind {
        NvCommandKind::Malformed { .. } => {}
        other => panic!("expected Malformed, got {:?}", other),
    }
}

#[test]
fn new_jump_decodes() {
    let h = hdr(NV_FLAG_NEW_JUMP | 0x0003_0000);
    assert_eq!(
        h.kind,
        NvCommandKind::NewJump {
            offset: 0x0003_0000
        }
    );
}

#[test]
fn call_decodes() {
    let h = hdr(NV_FLAG_CALL | 0x0010_1000);
    assert_eq!(
        h.kind,
        NvCommandKind::Call {
            offset: 0x0010_1000
        }
    );
}

#[test]
fn return_decodes() {
    let h = hdr(NV_FLAG_RETURN);
    assert_eq!(h.kind, NvCommandKind::Return);
}

#[test]
fn return_with_extra_bits_is_malformed() {
    let h = hdr(NV_FLAG_RETURN | 0x0004_0000);
    match h.kind {
        NvCommandKind::Malformed { raw } => {
            assert_eq!(raw, NV_FLAG_RETURN | 0x0004_0000);
        }
        _ => panic!("expected Malformed, got {:?}", h.kind),
    }
}

#[test]
fn malformed_with_bit_31_set() {
    let h = hdr(0x8000_0000);
    match h.kind {
        NvCommandKind::Malformed { raw } => assert_eq!(raw, 0x8000_0000),
        _ => panic!("expected Malformed"),
    }
}

#[test]
fn malformed_with_reserved_bit_16_set() {
    let h = hdr(0x0001_0000);
    match h.kind {
        NvCommandKind::Malformed { raw } => assert_eq!(raw, 0x0001_0000),
        _ => panic!("expected Malformed"),
    }
}

#[test]
fn jump_plus_new_jump_is_malformed() {
    let h = hdr(NV_FLAG_JUMP | NV_FLAG_NEW_JUMP);
    match h.kind {
        NvCommandKind::Malformed { .. } => {}
        other => panic!("expected Malformed, got {:?}", other),
    }
}

#[test]
fn return_plus_call_decodes_as_call_per_hardware() {
    let h = hdr(NV_FLAG_RETURN | NV_FLAG_CALL);
    match h.kind {
        NvCommandKind::Call { offset } => {
            assert_eq!(offset, NV_FLAG_RETURN & NV_CALL_OFFSET_MASK);
        }
        other => panic!("expected Call, got {:?}", other),
    }
}

#[test]
fn return_plus_new_jump_decodes_as_new_jump_per_hardware() {
    let h = hdr(NV_FLAG_RETURN | NV_FLAG_NEW_JUMP);
    match h.kind {
        NvCommandKind::NewJump { offset } => {
            assert_eq!(offset, NV_FLAG_RETURN & NV_NEW_JUMP_OFFSET_MASK);
        }
        other => panic!("expected NewJump, got {:?}", other),
    }
}

#[test]
fn call_with_bit_31_set_decodes_as_call_per_hardware() {
    let h = hdr(0x8000_0000 | NV_FLAG_CALL);
    match h.kind {
        NvCommandKind::Call { offset } => {
            assert_eq!(offset, 0);
        }
        other => panic!("expected Call, got {:?}", other),
    }
}

#[test]
fn call_with_jump_flag_decodes_as_call_per_hardware() {
    let h = hdr(NV_FLAG_JUMP | NV_FLAG_CALL);
    match h.kind {
        NvCommandKind::Call { offset } => {
            assert_eq!(offset, NV_FLAG_JUMP & NV_CALL_OFFSET_MASK);
        }
        other => panic!("expected Call, got {:?}", other),
    }
}

#[test]
fn bit_31_alone_is_malformed() {
    let h = hdr(0x8000_0000);
    match h.kind {
        NvCommandKind::Malformed { raw } => assert_eq!(raw, 0x8000_0000),
        other => panic!("expected Malformed, got {:?}", other),
    }
}

#[test]
fn return_bit_with_stray_method_is_malformed() {
    let h = hdr(NV_FLAG_RETURN | 0x0000_0050);
    match h.kind {
        NvCommandKind::Malformed { raw } => {
            assert_eq!(raw, NV_FLAG_RETURN | 0x0000_0050);
        }
        other => panic!("expected Malformed, got {:?}", other),
    }
}

#[test]
fn table_new_is_empty() {
    let t = NvMethodTable::new();
    assert_eq!(t.len(), 0);
    assert!(t.is_empty());
}

#[test]
fn table_lookup_unregistered_method_returns_none() {
    let t = NvMethodTable::new();
    assert!(t.lookup(NV406E_SEMAPHORE_OFFSET).is_none());
    assert!(t.lookup(NV4097_GET_REPORT).is_none());
    assert!(t.lookup(GCM_FLIP_COMMAND).is_none());
}

fn fresh_state() -> (RsxFifoCursor, u32, Vec<Effect>) {
    (RsxFifoCursor::new(), 0u32, Vec::new())
}

fn ctx_for<'a>(
    cursor: &'a mut RsxFifoCursor,
    sem_offset: &'a mut u32,
    emitted: &'a mut Vec<Effect>,
) -> NvDispatchContext<'a> {
    NvDispatchContext {
        cursor,
        sem_offset,
        emitted,
        now: GuestTicks::ZERO,
    }
}

fn noop_handler(_ctx: &mut NvDispatchContext<'_>, _args: &[u32]) {}

#[test]
fn table_register_inserts_and_returns_none_on_fresh_slot() {
    let mut t = NvMethodTable::new();
    let prior = t.register(NV406E_SEMAPHORE_OFFSET, noop_handler);
    assert!(prior.is_none());
    assert_eq!(t.len(), 1);
    assert!(t.lookup(NV406E_SEMAPHORE_OFFSET).is_some());
}

#[test]
fn table_register_replaces_prior_handler() {
    fn handler_a(_ctx: &mut NvDispatchContext<'_>, _args: &[u32]) {}
    fn handler_b(_ctx: &mut NvDispatchContext<'_>, _args: &[u32]) {}
    let mut t = NvMethodTable::new();
    t.register(NV406E_SEMAPHORE_OFFSET, handler_a);
    let prior = t.register(NV406E_SEMAPHORE_OFFSET, handler_b);
    assert!(prior.is_some());
    assert_eq!(t.len(), 1, "same slot replaced, not appended");
}

#[test]
fn table_register_unique_succeeds_on_fresh_slot() {
    let mut t = NvMethodTable::new();
    let result = t.register_unique(NV406E_SEMAPHORE_OFFSET, noop_handler);
    assert!(result.is_ok());
    assert_eq!(t.len(), 1);
}

#[test]
fn table_register_unique_fails_on_collision_with_method_id() {
    fn handler_a(_ctx: &mut NvDispatchContext<'_>, _args: &[u32]) {}
    fn handler_b(_ctx: &mut NvDispatchContext<'_>, _args: &[u32]) {}
    let mut t = NvMethodTable::new();
    t.register_unique(NV406E_SEMAPHORE_OFFSET, handler_a)
        .unwrap();
    let err = t
        .register_unique(NV406E_SEMAPHORE_OFFSET, handler_b)
        .expect_err("collision must surface");
    assert_eq!(
        err.method, NV406E_SEMAPHORE_OFFSET,
        "error carries the offending method id"
    );
    assert_eq!(t.len(), 1, "failed unique-registration does not mutate");
}

// Reset TRUN_*_CALLS before each test that reuses trun_*_handler.
use std::sync::atomic::{AtomicU32, Ordering};
static TRUN_A_CALLS: AtomicU32 = AtomicU32::new(0);
static TRUN_B_CALLS: AtomicU32 = AtomicU32::new(0);
fn trun_a_handler(_ctx: &mut NvDispatchContext<'_>, _args: &[u32]) {
    TRUN_A_CALLS.fetch_add(1, Ordering::SeqCst);
}
fn trun_b_handler(_ctx: &mut NvDispatchContext<'_>, _args: &[u32]) {
    TRUN_B_CALLS.fetch_add(1, Ordering::SeqCst);
}

#[test]
fn table_register_unique_does_not_overwrite_on_failure() {
    // Side-channel check: a lookup-returns-Some assertion would
    // pass even if the overwrite had succeeded, so observe which
    // handler actually fires via its counter.
    TRUN_A_CALLS.store(0, Ordering::SeqCst);
    TRUN_B_CALLS.store(0, Ordering::SeqCst);

    let mut t = NvMethodTable::new();
    t.register_unique(NV406E_SEMAPHORE_OFFSET, trun_a_handler)
        .unwrap();
    let _ = t.register_unique(NV406E_SEMAPHORE_OFFSET, trun_b_handler);

    let h = t
        .lookup(NV406E_SEMAPHORE_OFFSET)
        .expect("handler still registered");
    let (mut cursor, mut sem_offset, mut emitted) = fresh_state();
    let mut ctx = ctx_for(&mut cursor, &mut sem_offset, &mut emitted);
    h(&mut ctx, &[]);

    assert_eq!(
        TRUN_A_CALLS.load(Ordering::SeqCst),
        1,
        "handler_a fired (prior handler preserved)"
    );
    assert_eq!(
        TRUN_B_CALLS.load(Ordering::SeqCst),
        0,
        "handler_b did NOT fire (overwrite attempted and rejected)"
    );
}

// --- NV406E semaphore handlers ---

#[test]
fn nv406e_semaphore_offset_stores_arg_into_sem_offset() {
    let (mut cursor, mut sem_offset, mut emitted) = fresh_state();
    let mut ctx = ctx_for(&mut cursor, &mut sem_offset, &mut emitted);
    nv406e_semaphore_offset(&mut ctx, &[0x1234_5678]);
    assert_eq!(sem_offset, 0x1234_5678);
    assert!(emitted.is_empty(), "offset handler emits nothing");
}

#[test]
fn nv406e_semaphore_offset_noop_on_empty_args() {
    let mut cursor = RsxFifoCursor::new();
    let mut sem_offset: u32 = 0xDEAD_BEEF;
    let mut emitted: Vec<Effect> = Vec::new();
    let mut ctx = ctx_for(&mut cursor, &mut sem_offset, &mut emitted);
    nv406e_semaphore_offset(&mut ctx, &[]);
    assert_eq!(sem_offset, 0xDEAD_BEEF);
}

#[test]
fn nv406e_semaphore_release_emits_label_write_with_current_offset() {
    let mut cursor = RsxFifoCursor::new();
    let mut sem_offset: u32 = 0x80;
    let mut emitted: Vec<Effect> = Vec::new();
    let mut ctx = ctx_for(&mut cursor, &mut sem_offset, &mut emitted);
    nv406e_semaphore_release(&mut ctx, &[0xAABB_CCDD]);
    assert_eq!(
        emitted.as_slice(),
        &[Effect::RsxLabelWrite {
            offset: 0x80,
            value: 0xAABB_CCDD,
        }]
    );
}

#[test]
fn nv406e_semaphore_release_noop_on_empty_args() {
    let (mut cursor, mut sem_offset, mut emitted) = fresh_state();
    let mut ctx = ctx_for(&mut cursor, &mut sem_offset, &mut emitted);
    nv406e_semaphore_release(&mut ctx, &[]);
    assert!(emitted.is_empty());
}

#[test]
fn nv406e_offset_release_pair_threads_offset_through_emitted_write() {
    let (mut cursor, mut sem_offset, mut emitted) = fresh_state();
    let mut ctx = ctx_for(&mut cursor, &mut sem_offset, &mut emitted);
    nv406e_semaphore_offset(&mut ctx, &[0x100]);
    nv406e_semaphore_release(&mut ctx, &[42]);
    assert_eq!(
        emitted.as_slice(),
        &[Effect::RsxLabelWrite {
            offset: 0x100,
            value: 42,
        }]
    );
}

#[test]
fn nv406e_release_without_prior_offset_uses_current_sem_offset() {
    let (mut cursor, mut sem_offset, mut emitted) = fresh_state();
    let mut ctx = ctx_for(&mut cursor, &mut sem_offset, &mut emitted);
    nv406e_semaphore_release(&mut ctx, &[9]);
    assert_eq!(
        emitted.as_slice(),
        &[Effect::RsxLabelWrite {
            offset: 0,
            value: 9,
        }]
    );
}

// --- NV406E_SET_REFERENCE handler ---

#[test]
fn nv406e_set_reference_stores_arg_into_cursor_current_reference() {
    let (mut cursor, mut sem_offset, mut emitted) = fresh_state();
    let mut ctx = ctx_for(&mut cursor, &mut sem_offset, &mut emitted);
    nv406e_set_reference(&mut ctx, &[0xCAFE_BABE]);
    assert_eq!(cursor.current_reference(), 0xCAFE_BABE);
    assert_eq!(sem_offset, 0, "reference handler must not touch sem_offset");
    assert!(emitted.is_empty(), "reference handler emits no effect");
}

#[test]
fn nv406e_set_reference_noop_on_empty_args() {
    let mut cursor = RsxFifoCursor::new();
    cursor.set_reference(0xDEAD_BEEF);
    let mut sem_offset = 0u32;
    let mut emitted: Vec<Effect> = Vec::new();
    let mut ctx = ctx_for(&mut cursor, &mut sem_offset, &mut emitted);
    nv406e_set_reference(&mut ctx, &[]);
    assert_eq!(cursor.current_reference(), 0xDEAD_BEEF);
}

#[test]
fn nv406e_set_reference_overwrites_prior_reference() {
    let mut cursor = RsxFifoCursor::new();
    cursor.set_reference(0x1111);
    let mut sem_offset = 0u32;
    let mut emitted: Vec<Effect> = Vec::new();
    let mut ctx = ctx_for(&mut cursor, &mut sem_offset, &mut emitted);
    nv406e_set_reference(&mut ctx, &[0x2222]);
    assert_eq!(
        cursor.current_reference(),
        0x2222,
        "later SET_REFERENCE overwrites earlier one; Sony reference semantics are last-writer-wins"
    );
}

// --- NV4097 GCM_FLIP_COMMAND handler ---

#[test]
fn nv4097_flip_buffer_emits_rsx_flip_request_with_buffer_index() {
    let (mut cursor, mut sem_offset, mut emitted) = fresh_state();
    let mut ctx = ctx_for(&mut cursor, &mut sem_offset, &mut emitted);
    nv4097_flip_buffer(&mut ctx, &[3]);
    assert_eq!(
        emitted.as_slice(),
        &[Effect::RsxFlipRequest { buffer_index: 3 }]
    );
}

#[test]
fn nv4097_flip_buffer_truncates_large_arg_to_buffer_index_byte() {
    let (mut cursor, mut sem_offset, mut emitted) = fresh_state();
    let mut ctx = ctx_for(&mut cursor, &mut sem_offset, &mut emitted);
    nv4097_flip_buffer(&mut ctx, &[0x1234_56FF]);
    assert_eq!(
        emitted.as_slice(),
        &[Effect::RsxFlipRequest { buffer_index: 0xFF }]
    );
}

#[test]
fn nv4097_flip_buffer_noop_on_empty_args() {
    let (mut cursor, mut sem_offset, mut emitted) = fresh_state();
    let mut ctx = ctx_for(&mut cursor, &mut sem_offset, &mut emitted);
    nv4097_flip_buffer(&mut ctx, &[]);
    assert!(emitted.is_empty());
}

// --- NV4097 GET_REPORT handler ---

fn ctx_with_time<'a>(
    cursor: &'a mut RsxFifoCursor,
    sem_offset: &'a mut u32,
    emitted: &'a mut Vec<Effect>,
    now: GuestTicks,
) -> NvDispatchContext<'a> {
    NvDispatchContext {
        cursor,
        sem_offset,
        emitted,
        now,
    }
}

#[test]
fn nv4097_get_report_emits_label_write_at_arg_offset() {
    let (mut cursor, mut sem_offset, mut emitted) = fresh_state();
    let mut ctx = ctx_with_time(
        &mut cursor,
        &mut sem_offset,
        &mut emitted,
        GuestTicks::new(0x1234),
    );
    let report_arg = 0x0100_0040u32;
    nv4097_get_report(&mut ctx, &[report_arg]);
    assert_eq!(
        emitted.as_slice(),
        &[Effect::RsxLabelWrite {
            offset: 0x0100_0040,
            value: 0x1234,
        }]
    );
}

#[test]
fn nv4097_get_report_uses_guest_ticks_as_value() {
    fn emit_with_time(ticks: u64) -> u32 {
        let (mut cursor, mut sem_offset, mut emitted) = fresh_state();
        let mut ctx = ctx_with_time(
            &mut cursor,
            &mut sem_offset,
            &mut emitted,
            GuestTicks::new(ticks),
        );
        nv4097_get_report(&mut ctx, &[0x0u32]);
        match emitted[0] {
            Effect::RsxLabelWrite { value, .. } => value,
            _ => panic!("expected RsxLabelWrite"),
        }
    }
    assert_eq!(emit_with_time(0), 0);
    assert_eq!(emit_with_time(1_000), 1_000);
    // Truncation to low 32 bits is the oracle's timestamp-slot contract.
    assert_eq!(emit_with_time(0x1_0000_0001), 1);
}

#[test]
fn nv4097_get_report_passes_full_u32_as_offset() {
    let (mut cursor, mut sem_offset, mut emitted) = fresh_state();
    let mut ctx = ctx_with_time(
        &mut cursor,
        &mut sem_offset,
        &mut emitted,
        GuestTicks::new(7),
    );
    nv4097_get_report(&mut ctx, &[0xFF12_3456u32]);
    assert_eq!(
        emitted.as_slice(),
        &[Effect::RsxLabelWrite {
            offset: 0xFF12_3456,
            value: 7,
        }]
    );
}

#[test]
fn nv4097_get_report_noop_on_empty_args() {
    let (mut cursor, mut sem_offset, mut emitted) = fresh_state();
    let mut ctx = ctx_with_time(
        &mut cursor,
        &mut sem_offset,
        &mut emitted,
        GuestTicks::new(5),
    );
    nv4097_get_report(&mut ctx, &[]);
    assert!(emitted.is_empty());
}

#[test]
fn register_nv4097_report_handler_inserts_address() {
    let mut t = NvMethodTable::new();
    register_nv4097_report_handler(&mut t).expect("fresh table");
    assert!(t.lookup(NV4097_GET_REPORT).is_some());
    assert_eq!(t.len(), 1);
}

#[test]
fn register_nv4097_report_handler_surfaces_collision() {
    let mut t = NvMethodTable::new();
    t.register_unique(NV4097_GET_REPORT, noop_handler).unwrap();
    let err =
        register_nv4097_report_handler(&mut t).expect_err("pre-existing registration must collide");
    assert_eq!(err.method, NV4097_GET_REPORT);
}

// --- NV4097 back-end semaphore handlers ---

#[test]
fn back_end_semaphore_value_swap_is_its_own_inverse() {
    for sample in [
        0x0000_0000u32,
        0xFFFF_FFFFu32,
        0x1122_3344u32,
        0xAA00_BBFFu32,
        0x1234_5678u32,
    ] {
        assert_eq!(
            back_end_semaphore_value_swap(back_end_semaphore_value_swap(sample)),
            sample,
            "swap is involutive on {sample:#010x}"
        );
    }
}

#[test]
fn back_end_semaphore_value_swap_exchanges_bytes_0_and_2() {
    assert_eq!(
        back_end_semaphore_value_swap(0x1122_3344),
        0x1144_3322,
        "bytes: [11 22 33 44] -> [11 44 33 22]"
    );
}

#[test]
fn nv4097_set_semaphore_offset_stores_arg() {
    let (mut cursor, mut sem_offset, mut emitted) = fresh_state();
    let mut ctx = ctx_for(&mut cursor, &mut sem_offset, &mut emitted);
    nv4097_set_semaphore_offset(&mut ctx, &[0x1234_5678]);
    assert_eq!(sem_offset, 0x1234_5678);
    assert!(emitted.is_empty());
}

#[test]
fn nv4097_set_semaphore_offset_noop_on_empty_args() {
    let mut cursor = RsxFifoCursor::new();
    let mut sem_offset = 0xDEAD_BEEFu32;
    let mut emitted: Vec<Effect> = Vec::new();
    let mut ctx = ctx_for(&mut cursor, &mut sem_offset, &mut emitted);
    nv4097_set_semaphore_offset(&mut ctx, &[]);
    assert_eq!(sem_offset, 0xDEAD_BEEF);
}

#[test]
fn nv4097_back_end_release_emits_label_write_with_byte_swap() {
    let mut cursor = RsxFifoCursor::new();
    let mut sem_offset: u32 = 0x20;
    let mut emitted: Vec<Effect> = Vec::new();
    let mut ctx = ctx_for(&mut cursor, &mut sem_offset, &mut emitted);
    // fifo_arg is the already-pre-swapped value the guest wrote.
    let fifo_arg = 0x1144_3322u32;
    nv4097_back_end_write_semaphore_release(&mut ctx, &[fifo_arg]);
    assert_eq!(
        emitted.as_slice(),
        &[Effect::RsxLabelWrite {
            offset: 0x20,
            value: 0x1122_3344,
        }]
    );
}

#[test]
fn nv4097_back_end_release_noop_on_empty_args() {
    let (mut cursor, mut sem_offset, mut emitted) = fresh_state();
    let mut ctx = ctx_for(&mut cursor, &mut sem_offset, &mut emitted);
    nv4097_back_end_write_semaphore_release(&mut ctx, &[]);
    assert!(emitted.is_empty());
}

#[test]
fn nv4097_back_end_pair_threads_offset_through_emitted_write_post_swap() {
    let (mut cursor, mut sem_offset, mut emitted) = fresh_state();
    let mut ctx = ctx_for(&mut cursor, &mut sem_offset, &mut emitted);
    nv4097_set_semaphore_offset(&mut ctx, &[0x10]);
    // 0xAADD_CCBB is the pre-swap of 0xAABB_CCDD.
    nv4097_back_end_write_semaphore_release(&mut ctx, &[0xAADD_CCBB]);
    assert_eq!(
        emitted.as_slice(),
        &[Effect::RsxLabelWrite {
            offset: 0x10,
            value: 0xAABB_CCDD,
        }]
    );
}

#[test]
fn register_nv4097_back_end_semaphore_handlers_inserts_both_addresses() {
    let mut t = NvMethodTable::new();
    register_nv4097_back_end_semaphore_handlers(&mut t).expect("fresh table");
    assert!(t.lookup(NV4097_SET_SEMAPHORE_OFFSET).is_some());
    assert!(t.lookup(NV4097_BACK_END_WRITE_SEMAPHORE_RELEASE).is_some());
    assert_eq!(t.len(), 2);
}

#[test]
fn register_nv4097_back_end_semaphore_handlers_surfaces_offset_collision() {
    let mut t = NvMethodTable::new();
    t.register_unique(NV4097_SET_SEMAPHORE_OFFSET, noop_handler)
        .unwrap();
    let err = register_nv4097_back_end_semaphore_handlers(&mut t)
        .expect_err("pre-existing OFFSET registration must collide");
    assert_eq!(err.method, NV4097_SET_SEMAPHORE_OFFSET);
    assert!(t.lookup(NV4097_BACK_END_WRITE_SEMAPHORE_RELEASE).is_none());
}

#[test]
fn register_nv4097_back_end_semaphore_handlers_surfaces_release_collision() {
    let mut t = NvMethodTable::new();
    t.register_unique(NV4097_BACK_END_WRITE_SEMAPHORE_RELEASE, noop_handler)
        .unwrap();
    let err = register_nv4097_back_end_semaphore_handlers(&mut t)
        .expect_err("pre-existing RELEASE registration must collide");
    assert_eq!(err.method, NV4097_BACK_END_WRITE_SEMAPHORE_RELEASE);
}

#[test]
fn register_nv4097_flip_handler_inserts_address() {
    let mut t = NvMethodTable::new();
    register_nv4097_flip_handler(&mut t).expect("fresh table");
    assert!(t.lookup(GCM_FLIP_COMMAND).is_some());
    assert_eq!(t.len(), 1);
}

#[test]
fn register_nv4097_flip_handler_surfaces_collision() {
    let mut t = NvMethodTable::new();
    t.register_unique(GCM_FLIP_COMMAND, noop_handler).unwrap();
    let err =
        register_nv4097_flip_handler(&mut t).expect_err("pre-existing registration must collide");
    assert_eq!(err.method, GCM_FLIP_COMMAND);
}

#[test]
fn register_nv406e_reference_handler_inserts_address() {
    let mut t = NvMethodTable::new();
    register_nv406e_reference_handler(&mut t).expect("fresh table");
    assert!(t.lookup(NV406E_SET_REFERENCE).is_some());
    assert_eq!(t.len(), 1);
}

#[test]
fn register_nv406e_reference_handler_surfaces_collision() {
    let mut t = NvMethodTable::new();
    t.register_unique(NV406E_SET_REFERENCE, noop_handler)
        .unwrap();
    let err = register_nv406e_reference_handler(&mut t)
        .expect_err("pre-existing registration must collide");
    assert_eq!(err.method, NV406E_SET_REFERENCE);
}

#[test]
fn register_nv406e_label_handlers_inserts_both_addresses() {
    let mut t = NvMethodTable::new();
    register_nv406e_label_handlers(&mut t).expect("fresh table");
    assert!(t.lookup(NV406E_SEMAPHORE_OFFSET).is_some());
    assert!(t.lookup(NV406E_SEMAPHORE_RELEASE).is_some());
    assert_eq!(t.len(), 2);
}

#[test]
fn register_nv406e_label_handlers_surfaces_offset_collision() {
    let mut t = NvMethodTable::new();
    t.register_unique(NV406E_SEMAPHORE_OFFSET, noop_handler)
        .unwrap();
    let err =
        register_nv406e_label_handlers(&mut t).expect_err("OFFSET already present must collide");
    assert_eq!(err.method, NV406E_SEMAPHORE_OFFSET);
    assert!(t.lookup(NV406E_SEMAPHORE_RELEASE).is_none());
}

#[test]
fn register_nv406e_label_handlers_surfaces_release_collision() {
    let mut t = NvMethodTable::new();
    t.register_unique(NV406E_SEMAPHORE_RELEASE, noop_handler)
        .unwrap();
    let err =
        register_nv406e_label_handlers(&mut t).expect_err("RELEASE already present must collide");
    assert_eq!(err.method, NV406E_SEMAPHORE_RELEASE);
    // Offset registers before the release collision, so it
    // remains in the table. The helper is not all-or-nothing.
    assert!(t.lookup(NV406E_SEMAPHORE_OFFSET).is_some());
}

#[test]
fn nv_constant_values_pin_rpcs3_lineage() {
    assert_eq!(NV406E_SET_REFERENCE, 0x0050);
    assert_eq!(NV406E_SEMAPHORE_OFFSET, 0x0064);
    assert_eq!(NV406E_SEMAPHORE_ACQUIRE, 0x0068);
    assert_eq!(NV406E_SEMAPHORE_RELEASE, 0x006C);
    assert_eq!(NV4097_NO_OPERATION, 0x0100);
    assert_eq!(NV4097_GET_REPORT, 0x1800);
    assert_eq!(GCM_FLIP_COMMAND, 0xFEAC);
    assert_eq!(NV_FLAG_NON_INCREMENT, 0x4000_0000);
    assert_eq!(NV_FLAG_JUMP, 0x2000_0000);
    assert_eq!(NV_FLAG_CALL, 0x0000_0002);
    assert_eq!(NV_FLAG_RETURN, 0x0002_0000);
    assert_eq!(NV_COUNT_SHIFT, 18);
}

#[test]
fn with_default_handlers_registers_exactly_the_expected_method_ids() {
    let expected = [
        NV406E_SEMAPHORE_OFFSET,
        NV406E_SEMAPHORE_RELEASE,
        NV406E_SET_REFERENCE,
        GCM_FLIP_COMMAND,
        NV4097_GET_REPORT,
        NV4097_SET_SEMAPHORE_OFFSET,
        NV4097_BACK_END_WRITE_SEMAPHORE_RELEASE,
    ];
    let table = NvMethodTable::with_default_handlers();
    assert_eq!(
        table.len(),
        expected.len(),
        "roster size shifted: expected {} handlers, found {}",
        expected.len(),
        table.len(),
    );
    for method in expected {
        assert!(
            table.lookup(method).is_some(),
            "expected method 0x{method:04x} missing from with_default_handlers roster",
        );
    }
}
