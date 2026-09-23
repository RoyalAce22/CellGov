//! Counterexamples the model-based properties found, pinned by name.

use super::*;

#[test]
fn a_later_partial_store_is_not_hidden_behind_an_older_covering_store() {
    let mut buf = StoreBuffer::new();
    buf.insert(0x100, 4, 0x1111_1111).expect("staged");
    buf.insert(0x102, 2, 0x2222).expect("staged");
    assert_eq!(buf.forward(0x100, 4), None);
    let mut out = [0u8; 4];
    buf.overlay_range(0x100, &mut out);
    assert_eq!(out, [0x11, 0x11, 0x22, 0x22]);
}

#[test]
fn an_older_partial_store_under_a_later_covering_store_still_forwards() {
    let mut buf = StoreBuffer::new();
    buf.insert(0x102, 2, 0x2222).expect("staged");
    buf.insert(0x100, 4, 0x1111_1111).expect("staged");
    assert_eq!(buf.forward(0x100, 4), Some(0x1111_1111));
}

#[test]
fn a_store_at_the_top_of_the_address_space_is_refused_when_it_wraps() {
    let mut buf = StoreBuffer::new();
    assert_eq!(
        buf.insert(u64::MAX - 1, 4, 0),
        Err(StoreRefusal::AddressWraps {
            addr: u64::MAX - 1,
            len: 4
        })
    );
    assert_eq!(
        buf.insert_conditional(u64::MAX, 1, 0, 0),
        Err(StoreRefusal::AddressWraps {
            addr: u64::MAX,
            len: 1
        })
    );
    assert!(buf.is_empty());
    // The last byte of the space has no representable exclusive end, so
    // the highest store of four bytes ends one byte short of it.
    assert_eq!(buf.insert(u64::MAX - 4, 4, 0xDEAD_BEEF), Ok(()));
    assert_eq!(buf.forward(u64::MAX - 4, 4), Some(0xDEAD_BEEF));
    assert_eq!(buf.forward(u64::MAX - 1, 4), None);
}

#[test]
fn a_full_buffer_is_refused_as_full() {
    let mut buf = StoreBuffer::new();
    for i in 0..CAPACITY {
        buf.insert(i as u64 * 4, 4, i as u128).expect("staged");
    }
    assert_eq!(buf.insert(0xFFFF, 4, 0), Err(StoreRefusal::Full));
    assert_eq!(
        buf.insert_conditional(0xFFFF, 4, 0, 0),
        Err(StoreRefusal::Full)
    );
    // The address check runs before the capacity check.
    assert_eq!(
        buf.insert(u64::MAX - 1, 4, 0),
        Err(StoreRefusal::AddressWraps {
            addr: u64::MAX - 1,
            len: 4
        })
    );
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "store width out of range")]
fn a_zero_width_store_trips_the_debug_invariant() {
    let mut buf = StoreBuffer::new();
    let _ = buf.insert(0x100, 0, 0);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "store width out of range")]
fn a_store_wider_than_the_payload_trips_the_debug_invariant() {
    let mut buf = StoreBuffer::new();
    let _ = buf.insert(0x100, 17, 0);
}
