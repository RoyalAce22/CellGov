//! Store-buffer insertion, forwarding, and capacity behavior.

use super::*;

#[test]
fn empty_buffer() {
    let buf = StoreBuffer::new();
    assert!(buf.is_empty());
    assert_eq!(buf.len(), 0);
    assert!(!buf.is_full());
    assert!(buf.forward(0, 4).is_none());
}

#[test]
fn insert_and_forward_u32() {
    let mut buf = StoreBuffer::new();
    let val = 0xDEADBEEF_u128;
    buf.insert(0x100, 4, val).expect("staged");
    assert_eq!(buf.len(), 1);

    let fwd = buf.forward(0x100, 4);
    assert_eq!(fwd, Some(0xDEADBEEF));
}

#[test]
fn insert_and_forward_u8() {
    let mut buf = StoreBuffer::new();
    buf.insert(0x200, 1, 0x42).expect("staged");
    let fwd = buf.forward(0x200, 1);
    assert_eq!(fwd, Some(0x42));
}

#[test]
fn insert_and_forward_u64() {
    let mut buf = StoreBuffer::new();
    let val = 0xCAFEBABE_DEADBEEF_u128;
    buf.insert(0x300, 8, val).expect("staged");
    let fwd = buf.forward(0x300, 8);
    assert_eq!(fwd, Some(0xCAFEBABE_DEADBEEF));
}

#[test]
fn forward_no_overlap_returns_none() {
    let mut buf = StoreBuffer::new();
    buf.insert(0x100, 4, 0xAA).expect("staged");
    assert!(buf.forward(0x200, 4).is_none());
}

#[test]
fn forward_partial_overlap_returns_none() {
    let mut buf = StoreBuffer::new();
    buf.insert(0x100, 2, 0xBBCC).expect("staged");
    assert!(buf.forward(0x100, 4).is_none());
}

#[test]
fn most_recent_store_wins() {
    let mut buf = StoreBuffer::new();
    buf.insert(0x100, 4, 0x11111111).expect("staged");
    buf.insert(0x100, 4, 0x22222222).expect("staged");
    let fwd = buf.forward(0x100, 4);
    assert_eq!(fwd, Some(0x22222222));
}

#[test]
fn wider_store_covers_narrower_load() {
    let mut buf = StoreBuffer::new();
    buf.insert(0x100, 8, 0x1122334455667788).expect("staged");
    let fwd = buf.forward(0x100, 4);
    assert_eq!(fwd, Some(0x11223344));
}

#[test]
fn wider_store_covers_offset_load() {
    let mut buf = StoreBuffer::new();
    buf.insert(0x100, 8, 0x1122334455667788).expect("staged");
    let fwd = buf.forward(0x104, 4);
    assert_eq!(fwd, Some(0x55667788));
}

#[test]
fn capacity_overflow_is_refused_as_full() {
    let mut buf = StoreBuffer::new();
    for i in 0..CAPACITY {
        buf.insert(i as u64 * 4, 4, i as u128).expect("staged");
    }
    assert!(buf.is_full());
    assert_eq!(buf.insert(0xFFFF, 4, 0), Err(StoreRefusal::Full));
}

#[test]
fn clear_resets_buffer() {
    let mut buf = StoreBuffer::new();
    buf.insert(0x100, 4, 0xAA).expect("staged");
    assert_eq!(buf.len(), 1);
    buf.clear();
    assert!(buf.is_empty());
    assert!(buf.forward(0x100, 4).is_none());
}

#[test]
fn flush_emits_effects_in_order() {
    let mut buf = StoreBuffer::new();
    buf.insert(0x100, 4, 0xAABBCCDD).expect("staged");
    buf.insert(0x200, 2, 0xEEFF).expect("staged");

    let mut effects = Vec::new();
    buf.flush(&mut effects, UnitId::new(0));
    assert_eq!(effects.len(), 2);
    assert!(buf.is_empty());

    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x100);
            assert_eq!(range.length(), 4);
            assert_eq!(bytes.bytes(), &[0xAA, 0xBB, 0xCC, 0xDD]);
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
    match &effects[1] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x200);
            assert_eq!(range.length(), 2);
            assert_eq!(bytes.bytes(), &[0xEE, 0xFF]);
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn has_store_in_range_detects_overlap() {
    let mut buf = StoreBuffer::new();
    buf.insert(0x100, 4, 0).expect("staged");
    assert!(buf.has_store_in_range(0x100, 0x200));
    assert!(buf.has_store_in_range(0x0, 0x104));
    assert!(!buf.has_store_in_range(0x200, 0x300));
    assert!(!buf.has_store_in_range(0x0, 0x100));
}

#[test]
fn overlay_range_patches_only_overlapping_bytes() {
    // Region holds 0x11..0x20; one buffered 4-byte store at
    // 0x104 overrides bytes 4..8 of the 16-byte window.
    let mut buf = StoreBuffer::new();
    buf.insert(0x104, 4, 0xDEAD_BEEFu128).expect("staged");
    let mut out = [
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
        0x1F,
    ];
    buf.overlay_range(0x100, &mut out);
    assert_eq!(
        out,
        [
            0x10, 0x11, 0x12, 0x13, // unchanged
            0xDE, 0xAD, 0xBE, 0xEF, // patched
            0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F,
        ]
    );
}

#[test]
fn overlay_range_later_store_wins_in_overlap() {
    let mut buf = StoreBuffer::new();
    buf.insert(0x100, 4, 0x1111_1111u128).expect("staged");
    buf.insert(0x102, 2, 0x2222u128).expect("staged"); // overlaps the upper half of the first
    let mut out = [0u8; 8];
    buf.overlay_range(0x100, &mut out);
    // First store: 0x11 0x11 0x11 0x11 at offset 0..4.
    // Second store: 0x22 0x22 at offset 2..4 -- overwrites.
    assert_eq!(out[0..4], [0x11, 0x11, 0x22, 0x22]);
    assert_eq!(out[4..8], [0, 0, 0, 0]);
}

#[test]
fn overlay_range_skips_entries_outside_window() {
    let mut buf = StoreBuffer::new();
    buf.insert(0x80, 4, 0xDEAD_BEEFu128).expect("staged"); // before window
    buf.insert(0x200, 4, 0xCAFE_BABEu128).expect("staged"); // after window
    let mut out = [0xAAu8; 16];
    buf.overlay_range(0x100, &mut out);
    assert_eq!(out, [0xAA; 16]);
}

#[test]
fn forward_u16_from_u32_store() {
    let mut buf = StoreBuffer::new();
    buf.insert(0x100, 4, 0xAABBCCDD).expect("staged");
    let fwd = buf.forward(0x100, 2);
    assert_eq!(fwd, Some(0xAABB));
    let fwd = buf.forward(0x102, 2);
    assert_eq!(fwd, Some(0xCCDD));
}

#[test]
fn forward_single_byte_from_u32_store() {
    let mut buf = StoreBuffer::new();
    buf.insert(0x100, 4, 0xAABBCCDD).expect("staged");
    assert_eq!(buf.forward(0x100, 1), Some(0xAA));
    assert_eq!(buf.forward(0x101, 1), Some(0xBB));
    assert_eq!(buf.forward(0x102, 1), Some(0xCC));
    assert_eq!(buf.forward(0x103, 1), Some(0xDD));
}

#[test]
fn insert_u16_vector_store() {
    let mut buf = StoreBuffer::new();
    let val = 0x0102030405060708090A0B0C0D0E0F10_u128;
    buf.insert(0x100, 16, val).expect("staged");
    let fwd = buf.forward(0x100, 16);
    assert_eq!(fwd, Some(val));
}

#[test]
fn flush_emits_conditional_entries_in_program_order() {
    let mut buf = StoreBuffer::new();
    buf.insert(0x100, 4, 0xAABBCCDD).expect("staged");
    buf.insert_conditional(0x200, 4, 0x11223344, 0)
        .expect("staged");
    buf.insert(0x300, 2, 0xEEFF).expect("staged");

    let mut effects = Vec::new();
    buf.flush(&mut effects, UnitId::new(0));
    assert!(buf.is_empty());
    assert_eq!(effects.len(), 3);
    match &effects[0] {
        Effect::SharedWriteIntent { range, .. } => {
            assert_eq!(range.start().raw(), 0x100);
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
    match &effects[1] {
        Effect::ConditionalStore { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x200);
            assert_eq!(bytes.bytes(), &0x11223344u32.to_be_bytes());
        }
        other => panic!("expected ConditionalStore, got {other:?}"),
    }
    match &effects[2] {
        Effect::SharedWriteIntent { range, .. } => {
            assert_eq!(range.start().raw(), 0x300);
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

fn acquire(line_addr: u64) -> Effect {
    Effect::ReservationAcquire {
        line_addr,
        source: UnitId::new(0),
    }
}

fn kind(e: &Effect) -> &'static str {
    match e {
        Effect::ReservationAcquire { .. } => "acquire",
        Effect::ConditionalStore { .. } => "conditional",
        Effect::SharedWriteIntent { .. } => "plain",
        _ => "other",
    }
}

#[test]
fn flush_places_a_conditional_entry_before_effects_emitted_after_it() {
    let mut buf = StoreBuffer::new();
    let mut effects = vec![acquire(0x200)];
    buf.insert(0x100, 4, 0xAABBCCDD).expect("staged");
    buf.insert_conditional(0x200, 4, 0x11223344, effects.len())
        .expect("staged");
    // A later lwarx in the same block: pushed straight into the
    // vector, after the conditional store's recorded slot.
    effects.push(acquire(0x300));
    buf.insert(0x300, 2, 0xEEFF).expect("staged");

    buf.flush(&mut effects, UnitId::new(0));
    assert!(buf.is_empty());
    let kinds: Vec<_> = effects.iter().map(kind).collect();
    assert_eq!(
        kinds,
        ["acquire", "plain", "conditional", "acquire", "plain"],
        "{effects:?}"
    );
    match &effects[2] {
        Effect::ConditionalStore { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x200);
            assert_eq!(bytes.bytes(), &0x11223344u32.to_be_bytes());
        }
        other => panic!("expected ConditionalStore, got {other:?}"),
    }
}

#[test]
fn flush_shifts_a_second_conditional_slot_by_the_first_group() {
    let mut buf = StoreBuffer::new();
    let mut effects = Vec::new();
    buf.insert_conditional(0x100, 4, 0x1, effects.len())
        .expect("staged");
    effects.push(acquire(0x200));
    buf.insert(0x180, 4, 0x2).expect("staged");
    buf.insert_conditional(0x200, 4, 0x3, effects.len())
        .expect("staged");
    effects.push(acquire(0x300));
    buf.insert(0x280, 4, 0x4).expect("staged");

    buf.flush(&mut effects, UnitId::new(0));
    let kinds: Vec<_> = effects.iter().map(kind).collect();
    assert_eq!(
        kinds,
        [
            "conditional",
            "acquire",
            "plain",
            "conditional",
            "acquire",
            "plain"
        ],
        "{effects:?}"
    );
    let addrs: Vec<u64> = effects
        .iter()
        .map(|e| match e {
            Effect::ReservationAcquire { line_addr, .. } => *line_addr,
            Effect::SharedWriteIntent { range, .. } | Effect::ConditionalStore { range, .. } => {
                range.start().raw()
            }
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(addrs, [0x100u64, 0x200, 0x180, 0x200, 0x300, 0x280]);
}

#[test]
fn overlay_range_applies_conditional_entry() {
    // Within a single step the context is frozen, so committed
    // memory has not yet absorbed the `ConditionalStore`. The
    // buffered conditional entry is the sole intra-step record
    // of those bytes -- a vector load overlapping the range must
    // see them.
    let mut buf = StoreBuffer::new();
    buf.insert_conditional(0x104, 4, 0xCAFE_BABE_u128, 0)
        .expect("staged");
    let mut out = [0xAAu8; 16];
    buf.overlay_range(0x100, &mut out);
    assert_eq!(out[0..4], [0xAA, 0xAA, 0xAA, 0xAA]);
    assert_eq!(out[4..8], [0xCA, 0xFE, 0xBA, 0xBE]);
    assert_eq!(out[8..16], [0xAA; 8]);
}

#[test]
fn overlay_range_entry_starts_before_window_extends_in() {
    // Store at 0xFE..0x102 -- straddles the low edge of a window
    // anchored at 0x100. Only the bytes >= base must be applied.
    let mut buf = StoreBuffer::new();
    buf.insert(0xFE, 4, 0x1122_3344_u128).expect("staged");
    let mut out = [0xAAu8; 8];
    buf.overlay_range(0x100, &mut out);
    // Entry value bytes (BE): 11 22 33 44 covering EAs FE FF 100 101.
    // The window starts at 100, so only bytes 0x33 (EA 100) and
    // 0x44 (EA 101) fall inside.
    assert_eq!(out[0..2], [0x33, 0x44]);
    assert_eq!(out[2..8], [0xAA; 6]);
}

#[test]
fn overlay_range_entry_starts_in_window_extends_past_end() {
    // Store at 0x106..0x10A -- straddles the high edge of the
    // 8-byte window at 0x100..0x108. Only bytes < base_end apply.
    let mut buf = StoreBuffer::new();
    buf.insert(0x106, 4, 0xDEAD_BEEF_u128).expect("staged");
    let mut out = [0xAAu8; 8];
    buf.overlay_range(0x100, &mut out);
    // Entry covers EAs 106 107 108 109 with bytes DE AD BE EF.
    // Only EAs 106 and 107 are inside the window.
    assert_eq!(out[0..6], [0xAA; 6]);
    assert_eq!(out[6..8], [0xDE, 0xAD]);
}
