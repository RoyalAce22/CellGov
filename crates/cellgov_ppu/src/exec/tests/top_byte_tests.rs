//! Loads and stores through a view over its own addresses that ends at
//! 2^64, as a captured memory window can. No effect can name the real
//! byte 2^64 - 1, so the helpers refuse an access that reaches it.

use super::memory_helpers::{buffer_store, load_ze, unmapped, LoadPort, Width};
use super::ExecuteVerdict;
use crate::store_buffer::StoreBuffer;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::RegionView;

const WINDOW: u64 = u64::MAX - 7;

/// The start and length of each read or write intent in `effects`.
fn ranges(effects: &[Effect]) -> Vec<(u64, u64)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedReadIntent { range, .. } | Effect::SharedWriteIntent { range, .. } => {
                Some((range.start().raw(), range.length()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_load_of_the_last_byte_of_a_window_at_the_top_is_refused_at_that_byte() {
    let mem = [0x11u8; 8];
    let views = [RegionView::plain(WINDOW, &mem)];
    let store_buf = StoreBuffer::new();
    let mut effects = Vec::new();
    {
        let mut port = LoadPort::new(&views, &store_buf, &mut effects, UnitId::new(0));
        assert_eq!(
            load_ze(&mut port, WINDOW, Width::B8),
            Err(unmapped(u64::MAX))
        );
        assert_eq!(load_ze(&mut port, WINDOW, Width::B4), Ok(0x1111_1111));
    }
    assert_eq!(ranges(&effects), vec![(WINDOW, 4)]);
}

#[test]
fn a_store_to_the_last_byte_of_a_window_at_the_top_is_refused_and_stages_nothing() {
    let mem = [0u8; 8];
    let views = [RegionView::plain(WINDOW, &mem)];
    let mut store_buf = StoreBuffer::new();
    assert_eq!(
        buffer_store(&mut store_buf, &views, u64::MAX - 3, 4, 0xAABB_CCDD),
        ExecuteVerdict::StoreFault(unmapped(u64::MAX))
    );
    assert!(store_buf.is_empty());
    assert_eq!(
        buffer_store(&mut store_buf, &views, WINDOW, 4, 0xAABB_CCDD),
        ExecuteVerdict::Continue
    );
    let mut effects = Vec::new();
    store_buf.flush(&mut effects, UnitId::new(0));
    assert_eq!(ranges(&effects), vec![(WINDOW, 4)]);
}
