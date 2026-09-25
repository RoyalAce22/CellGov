//! The sync-state lane index, lane contributions, mixer terms and the
//! lane map.

use super::*;

/// Computed outside the crate from the SplitMix64 key stream of
/// `SYNC_KEY_SEED`.
#[test]
fn lane_wire_format_golden() {
    assert_eq!(additive_key(), 0x9fbb_01aa_12b5_0999_234d_a09c_663e_b184);
    assert_eq!(
        contribution(LaneIndex::new(1, 0, 0, 0), 3),
        0x0aee_936f_b502_21c9_7a57_f65c_d91f_ade3
    );
}

#[test]
fn the_index_packs_each_component_into_its_field() {
    assert_eq!(
        LaneIndex::new(1, 2, 3, 4).packed(),
        Some((1 << 55) | (2 << 23) | (3 << 15) | 4)
    );
    let below_top = LaneIndex::new(u8::MAX, u64::from(u32::MAX), u8::MAX, (1 << 15) - 2);
    assert_eq!(below_top.packed(), Some((1 << 63) - 2));
    let top = LaneIndex::new(u8::MAX, u64::from(u32::MAX), u8::MAX, (1 << 15) - 1);
    assert_eq!(
        top.packed(),
        None,
        "index + 1 would reach the key of index 0"
    );
    assert_ne!(contribution(top, 1), 0);
    assert_ne!(contribution(top, 1), contribution(below_top, 1));
}

#[test]
fn each_component_moves_the_packed_index() {
    let base = LaneIndex::new(7, 100, 5, 9);
    let moved = [
        LaneIndex::new(8, 100, 5, 9),
        LaneIndex::new(7, 101, 5, 9),
        LaneIndex::new(7, 100, 6, 9),
        LaneIndex::new(7, 100, 5, 10),
    ];
    for m in moved {
        assert_ne!(m.packed(), base.packed(), "{m:?}");
        assert_ne!(contribution(m, 1), contribution(base, 1), "{m:?}");
    }
}

#[test]
fn an_out_of_range_component_does_not_pack_but_still_contributes() {
    let wide_object = LaneIndex::new(1, 1 << 32, 0, 0);
    let wide_slot = LaneIndex::new(1, 0, 0, 1 << 15);
    assert_eq!(wide_object.packed(), None);
    assert_eq!(wide_slot.packed(), None);
    assert_ne!(contribution(wide_object, 1), 0);
    assert_ne!(contribution(wide_object, 1), contribution(wide_slot, 1));
    assert_ne!(contribution(wide_object, 1), contribution(wide_object, 2));
    assert_ne!(
        contribution(wide_object, 1),
        contribution(LaneIndex::new(1, 0, 0, 0), 1)
    );
}

#[test]
fn a_zero_lane_contributes_nothing() {
    assert_eq!(contribution(LaneIndex::new(1, 2, 3, 4), 0), 0);
    assert_eq!(contribution(LaneIndex::new(1, 1 << 40, 3, 4), 0), 0);
}

#[test]
fn a_bytes_term_moves_with_its_tag_key_and_content() {
    let base = bytes_term(4, &[1], b"/dev_hdd0/a");
    assert_ne!(base, bytes_term(5, &[1], b"/dev_hdd0/a"));
    assert_ne!(base, bytes_term(4, &[2], b"/dev_hdd0/a"));
    assert_ne!(base, bytes_term(4, &[1], b"/dev_hdd0/b"));
    assert_ne!(bytes_term(4, &[], b"a"), bytes_term(4, &[], b"a\0"));
    assert_ne!(bytes_term(4, &[], b""), 0);
    assert_eq!(base as u64, 0, "a term lives in the high half");
}

#[derive(Debug, Clone, PartialEq)]
struct Word(u64);

impl LaneValue for Word {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.0);
    }
}

fn map() -> LaneMap<u64, Word> {
    LaneMap::new(1, |k| k)
}

/// The partial the map keeps against the one it rebuilds, read in
/// release too, where `partial()` does not check itself.
fn kept(m: &LaneMap<u64, Word>) -> u128 {
    assert_eq!(m.partial, m.partial_from_scratch());
    m.partial
}

#[test]
fn an_entry_contributes_its_presence_lane_and_its_value_lanes() {
    let mut m = map();
    assert_eq!(kept(&m), 0);
    m.insert(3, Word(0));
    assert_eq!(kept(&m), contribution(LaneIndex::new(1, 3, 0, 0), 1));
    m.insert(3, Word(7));
    assert_eq!(
        kept(&m),
        contribution(LaneIndex::new(1, 3, 0, 0), 1)
            .wrapping_add(contribution(LaneIndex::new(1, 3, 1, 0), 7))
    );
}

#[test]
fn every_changing_method_keeps_the_partial() {
    let mut m = map();
    for k in 0..6 {
        m.insert(k, Word(k * 10));
        kept(&m);
    }
    assert_eq!(m.insert(2, Word(99)), Some(Word(20)));
    kept(&m);
    assert_eq!(m.remove(4), Some(Word(40)));
    kept(&m);
    assert_eq!(m.remove(4), None);
    kept(&m);
    assert_eq!(m.pop_first(), Some((0, Word(0))));
    kept(&m);
    m.get_mut(1).unwrap().0 = 11;
    kept(&m);
    m.retain(|k, v| {
        v.0 += 1;
        k != 3
    });
    kept(&m);
    assert_eq!(m.get(5), Some(&Word(51)));
    m.clear();
    assert_eq!(kept(&m), 0);
}

#[test]
fn a_change_through_the_guard_moves_the_partial() {
    let mut m = map();
    m.insert(1, Word(5));
    let before = kept(&m);
    m.get_mut(1).unwrap().0 = 6;
    assert_ne!(kept(&m), before);
    m.get_mut(1).unwrap().0 = 5;
    assert_eq!(kept(&m), before);
}

#[test]
fn a_slot_base_moves_every_lane_of_the_map() {
    let mut a = map();
    let mut b = map().with_slot_base(1);
    a.insert(2, Word(8));
    b.insert(2, Word(8));
    assert_ne!(kept(&a), kept(&b));
    assert_eq!(
        kept(&b),
        contribution(LaneIndex::new(1, 2, 0, 1), 1)
            .wrapping_add(contribution(LaneIndex::new(1, 2, 1, 1), 8))
    );
}

#[test]
fn equal_entries_give_equal_partials_whatever_the_insertion_order() {
    let mut a = map();
    let mut b = map();
    a.insert(1, Word(1));
    a.insert(2, Word(2));
    b.insert(2, Word(2));
    b.insert(1, Word(9));
    b.insert(1, Word(1));
    assert_eq!(kept(&a), kept(&b));
}
