use super::*;
use crate::host::test_support::primary_attrs;
use crate::ppu_thread::PpuThreadId;
use cellgov_event::UnitId;

#[test]
fn a_transient_unit_aliases_to_a_spawned_childs_thread_not_the_primary() {
    let mut host = Lv2Host::new();
    host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
    let child_primary = UnitId::new(5);
    let child_thread = host
        .ppu_threads_mut()
        .create(child_primary, primary_attrs())
        .expect("thread ids available");
    assert_ne!(child_thread, PpuThreadId::PRIMARY);

    let transient = UnitId::new(9);
    assert!(host.alias_unit_to_thread_of(transient, child_primary));
    assert_eq!(host.ppu_thread_id_for_unit(transient), Some(child_thread));

    assert!(host.drop_ppu_thread_alias(transient));
    assert_eq!(host.ppu_thread_id_for_unit(transient), None);
}

#[test]
fn aliasing_to_a_unit_without_a_thread_record_is_refused() {
    let mut host = Lv2Host::new();
    host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
    assert!(!host.alias_unit_to_thread_of(UnitId::new(9), UnitId::new(7)));
    assert_eq!(host.ppu_thread_id_for_unit(UnitId::new(9)), None);
}

#[test]
fn an_already_mapped_unit_cannot_be_re_aliased() {
    let mut host = Lv2Host::new();
    host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
    let child_primary = UnitId::new(5);
    host.ppu_threads_mut()
        .create(child_primary, primary_attrs())
        .expect("thread ids available");
    assert!(!host.alias_unit_to_thread_of(child_primary, UnitId::new(0)));
    assert!(!host.alias_unit_to_thread_of(UnitId::new(0), child_primary));
}
