//! Store-buffer entries at the top of the effective address space, and a
//! flush at the pieces its caller gives.

use super::*;

/// The write intents and conditional stores of `effects`, each as its
/// start, its bytes and whether it is conditional; a read intent as its
/// start and no bytes.
fn shapes(effects: &[Effect]) -> Vec<(u64, Vec<u8>, bool)> {
    effects
        .iter()
        .map(|e| match e {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                (range.start().raw(), bytes.bytes().to_vec(), false)
            }
            Effect::ConditionalStore { range, bytes, .. } => {
                (range.start().raw(), bytes.bytes().to_vec(), true)
            }
            Effect::SharedReadIntent { range, .. } => (range.start().raw(), Vec::new(), false),
            other => panic!("unexpected effect {other:?}"),
        })
        .collect()
}

fn read_at(at: u64) -> Effect {
    Effect::SharedReadIntent {
        range: ByteRange::new(GuestAddr::new(at), 1).unwrap(),
        source: UnitId::new(0),
    }
}

#[test]
fn an_effective_entry_that_ends_at_the_top_forwards_and_reaches_its_last_byte() {
    let mut buf = StoreBuffer::new();
    assert_eq!(
        buf.stage_effective(u64::MAX - 3, 4, 0xAABB_CCDD, None),
        Ok(())
    );
    assert_eq!(buf.forward(u64::MAX - 3, 4), Some(0xAABB_CCDD));
    assert_eq!(buf.forward(u64::MAX, 1), Some(0xDD));
    assert!(buf.has_store_in_range(u64::MAX - 1, u64::MAX));
    let mut out = [0u8; 2];
    buf.overlay_range(u64::MAX - 1, &mut out);
    assert_eq!(out, [0xCC, 0xDD]);
}

#[test]
fn an_effective_entry_that_runs_one_byte_past_the_top_is_refused() {
    let mut buf = StoreBuffer::new();
    assert_eq!(
        buf.stage_effective(u64::MAX - 2, 4, 0, None),
        Err(StoreRefusal::AddressWraps {
            addr: u64::MAX - 2,
            len: 4
        })
    );
    assert!(buf.is_empty());
}

/// A flush gives no `ByteRange` for the last byte of the address space.
#[test]
fn a_store_or_conditional_store_that_reaches_the_last_byte_is_refused() {
    let mut buf = StoreBuffer::new();
    let refused = Err(StoreRefusal::AddressWraps {
        addr: u64::MAX - 3,
        len: 4,
    });
    assert_eq!(buf.insert(u64::MAX - 3, 4, 0), refused);
    assert_eq!(buf.insert_conditional(u64::MAX - 3, 4, 0, 0), refused);
    assert!(buf.is_empty());
    assert_eq!(buf.insert(u64::MAX - 4, 4, 0), Ok(()));
}

// A debug build panics on the zero width at the width assertion.
#[cfg(not(debug_assertions))]
#[test]
fn a_zero_width_entry_at_0_forwards_and_overlays_nothing() {
    let mut buf = StoreBuffer::new();
    buf.insert(0x1000, 4, 0xAABB_CCDD).unwrap();
    buf.insert(0, 0, 0).unwrap();
    assert_eq!(buf.forward(0x1000, 4), Some(0xAABB_CCDD));
    let mut out = [0x11u8; 8];
    buf.overlay_range(0xFFC, &mut out);
    assert_eq!(out, [0x11, 0x11, 0x11, 0x11, 0xAA, 0xBB, 0xCC, 0xDD]);
}

/// [PPC-Book1 p:15 s:1.12.2] In 64-bit mode the effective address arithmetic wraps from 2^64 - 1 to 0.
#[test]
fn an_overlay_window_that_wraps_takes_the_entries_on_both_sides_of_0() {
    let mut buf = StoreBuffer::new();
    buf.stage_effective(u64::MAX - 1, 2, 0xAABB, None).unwrap();
    buf.stage_effective(0, 2, 0xCCDD, None).unwrap();
    let mut out = [0u8; 4];
    buf.overlay_range(u64::MAX - 1, &mut out);
    assert_eq!(out, [0xAA, 0xBB, 0xCC, 0xDD]);
}

#[test]
fn a_flush_publishes_each_entry_at_its_pieces_and_each_conditional_store_at_its_slot() {
    // A conditional store follows each of the plain stores at 0x100 and
    // 0x300, and each conditional store ran after one more read intent.
    // A third plain store follows the last conditional store.
    let mut buf = StoreBuffer::new();
    buf.stage_effective(0x100, 4, 0x1122_3344, None).unwrap();
    buf.stage_effective(0x200, 4, 0x55, Some(1)).unwrap();
    buf.stage_effective(0x300, 4, 0x5566_7788, None).unwrap();
    buf.stage_effective(0x400, 4, 0x66, Some(2)).unwrap();
    buf.stage_effective(0x500, 4, 0x99AA_BBCC, None).unwrap();
    let mut effects = vec![read_at(0xA0), read_at(0xA1), read_at(0xA2)];
    // Each plain store splits after its second byte; each piece moves up
    // by 0x1000.
    buf.flush_pieces(&mut effects, UnitId::new(0), |addr, len| {
        if len == 4 && addr % 0x200 == 0x100 {
            vec![(addr + 0x1000, 0, 2), (addr + 0x1002, 2, 2)]
        } else {
            vec![(addr + 0x1000, 0, u64::from(len))]
        }
    });
    assert_eq!(
        shapes(&effects),
        vec![
            (0xA0, vec![], false),
            (0x1100, vec![0x11, 0x22], false),
            (0x1102, vec![0x33, 0x44], false),
            (0x1200, vec![0, 0, 0, 0x55], true),
            (0xA1, vec![], false),
            (0x1300, vec![0x55, 0x66], false),
            (0x1302, vec![0x77, 0x88], false),
            (0x1400, vec![0, 0, 0, 0x66], true),
            (0xA2, vec![], false),
            (0x1500, vec![0x99, 0xAA], false),
            (0x1502, vec![0xBB, 0xCC], false),
        ]
    );
    assert!(buf.is_empty());
}
