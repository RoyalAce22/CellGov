use super::*;

fn table_with(ids: &[u32]) -> RawSpuTable {
    let mut table = RawSpuTable::new();
    for &id in ids {
        table.insert(id, UnitId::new(100 + u64::from(id)));
    }
    table
}

#[test]
fn a_new_raw_spu_takes_the_lowest_free_id_under_the_limit() {
    let mut table = RawSpuTable::new();
    assert_eq!(table.free_id(0), Err(RawSpuError::Exhausted));
    assert_eq!(table.free_id(2), Ok(0));
    table.insert(0, UnitId::new(1));
    table.insert(1, UnitId::new(2));
    assert_eq!(table.free_id(2), Err(RawSpuError::Exhausted));
    assert_eq!(table.free_id(5), Ok(2));
    table.remove(0).unwrap();
    assert_eq!(table.free_id(2), Ok(0));
}

#[test]
fn a_removed_raw_spu_is_unknown() {
    let mut table = table_with(&[0]);
    assert_eq!(table.remove(0).unwrap().unit(), UnitId::new(100));
    assert_eq!(table.remove(0), Err(RawSpuError::Unknown));
    assert_eq!(table.cfg(0), Err(RawSpuError::Unknown));
    assert_eq!(table.recover_page_fault(0), Err(RawSpuError::Unknown));
}

#[test]
fn an_interrupt_tag_is_one_per_reachable_class() {
    let mut table = table_with(&[0, 1]);
    let tag0 = table.create_interrupt_tag(0, 0).unwrap();
    assert_eq!(
        table.create_interrupt_tag(0, 0),
        Err(RawSpuError::TagExists { class: 0 })
    );
    assert_eq!(
        table.create_interrupt_tag(0, 1),
        Err(RawSpuError::InvalidClass { class: 1 })
    );
    let tag2 = table.create_interrupt_tag(0, 2).unwrap();
    assert_eq!(tag0 & 0xFF00_0000, raw_spu::INTERRUPT_TAG_KIND);
    assert_eq!(tag2 - tag0, raw_spu::INTERRUPT_TAG_STEP);
    assert_ne!(table.create_interrupt_tag(1, 0).unwrap(), tag0);
    assert_eq!(table.create_interrupt_tag(7, 0), Err(RawSpuError::Unknown));
}

#[test]
fn the_class_is_checked_before_the_id() {
    let mut table = table_with(&[0]);
    let bad_class = Err(RawSpuError::InvalidClass { class: 1 });
    assert_eq!(table.check_class(7, 1), bad_class);
    assert_eq!(table.set_int_mask(7, 1, 0), bad_class);
    assert_eq!(table.int_mask(7, 1), bad_class.map(|()| 0));
    assert_eq!(table.clear_int_stat(7, 1, 0), bad_class);
    assert_eq!(table.int_stat(7, 1), bad_class.map(|()| 0));
    assert_eq!(table.create_interrupt_tag(7, 1), bad_class.map(|()| 0));
    assert_eq!(table.set_int_mask(7, 0, 0), Err(RawSpuError::Unknown));
}

#[test]
fn masks_are_kept_per_class() {
    let mut table = table_with(&[0]);
    table.set_int_mask(0, 0, 0x7).unwrap();
    table.set_int_mask(0, 2, 0xF).unwrap();
    assert_eq!(table.int_mask(0, 0), Ok(0x7));
    assert_eq!(table.int_mask(0, 2), Ok(0xF));
}

#[test]
fn clearing_a_status_leaves_the_bits_it_does_not_name() {
    let mut table = table_with(&[0]);
    table.spus.get_mut(0).unwrap().stats[1] = 0x13;
    table.clear_int_stat(0, 2, 0x2).unwrap();
    assert_eq!(table.int_stat(0, 2), Ok(0x11));
    assert_eq!(table.int_stat(0, 0), Ok(0));
}

#[test]
fn the_cfg_keeps_its_two_signal_mode_bits() {
    let mut table = table_with(&[0]);
    table.set_cfg(0, 3).unwrap();
    assert_eq!(table.cfg(0), Ok(3));
    table.set_cfg(0, 4).unwrap();
    assert_eq!(table.cfg(0), Ok(0));
}

#[test]
fn no_raw_spu_is_stopped_by_a_page_fault() {
    let table = table_with(&[0]);
    assert_eq!(table.recover_page_fault(0), Err(RawSpuError::NoPageFault));
}

#[test]
fn an_empty_table_adds_nothing_to_the_sync_state() {
    let table = RawSpuTable::new();
    assert_eq!(table.sync_partial(), 0);
}

#[test]
fn every_raw_spu_field_reaches_the_sync_state() {
    let mut table = table_with(&[0]);
    let mut seen = vec![table.sync_partial()];
    table.create_interrupt_tag(0, 2).unwrap();
    seen.push(table.sync_partial());
    table.set_int_mask(0, 0, 1).unwrap();
    seen.push(table.sync_partial());
    table.spus.get_mut(0).unwrap().stats[0] = 1;
    seen.push(table.sync_partial());
    table.set_cfg(0, 1).unwrap();
    seen.push(table.sync_partial());
    table.spus.get_mut(0).unwrap().puint_mb = 1;
    seen.push(table.sync_partial());
    let mut distinct = seen.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), seen.len(), "{seen:x?}");
    assert_eq!(table.sync_partial(), table.sync_partial_from_scratch());
}

#[test]
fn the_tag_counter_reaches_the_sync_state() {
    let mut a = table_with(&[0]);
    let b = table_with(&[0]);
    a.create_interrupt_tag(0, 0).unwrap();
    a.remove(0).unwrap();
    a.insert(0, UnitId::new(100));
    assert_ne!(a.sync_partial(), b.sync_partial());
}
