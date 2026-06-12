//! Lv2Host state-hash sensitivity: PPU table, TLS template, hold counts, child stacks, and firmware identity each shift the hash deterministically.

use super::*;
use crate::host::test_support::primary_attrs;
use crate::ppu_thread::TlsTemplate;
use cellgov_event::UnitId;

#[test]
fn state_hash_unchanged_when_ppu_table_empty() {
    let fresh = Lv2Host::new();
    assert_eq!(fresh.state_hash(), Lv2Host::new().state_hash());
}

#[test]
fn state_hash_changes_after_primary_seed() {
    let pre_seed = Lv2Host::new().state_hash();
    let mut seeded = Lv2Host::new();
    seeded.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
    assert_ne!(pre_seed, seeded.state_hash());
}

#[test]
fn state_hash_unchanged_when_tls_template_empty() {
    let fresh = Lv2Host::new();
    assert_eq!(fresh.state_hash(), Lv2Host::new().state_hash());
}

#[test]
fn state_hash_changes_when_holds_inserted_then_returns_to_baseline() {
    let mut host = Lv2Host::new();
    host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
    let baseline = host.state_hash();
    let tid = host.ppu_thread_id_for_unit(UnitId::new(0)).unwrap();
    host.lwmutex_holds_inc(tid);
    assert_ne!(baseline, host.state_hash());
    host.lwmutex_holds_dec(tid);
    assert_eq!(baseline, host.state_hash());
}

#[test]
fn state_hash_changes_after_tls_template_set() {
    let pre = Lv2Host::new().state_hash();
    let mut host = Lv2Host::new();
    host.set_tls_template(TlsTemplate::new(vec![0x11, 0x22], 0x80, 0x10, 0x1000));
    assert_ne!(pre, host.state_hash());
}

#[test]
fn state_hash_unchanged_when_no_child_stack_allocated() {
    let fresh = Lv2Host::new();
    assert_eq!(fresh.state_hash(), Lv2Host::new().state_hash());
}

#[test]
fn state_hash_changes_after_child_stack_allocated() {
    let pre = Lv2Host::new().state_hash();
    let mut host = Lv2Host::new();
    let _ = host.allocate_child_stack(0x10_000, 0x10).unwrap();
    assert_ne!(pre, host.state_hash());
}

#[test]
fn state_hash_changes_after_firmware_identity_set() {
    let pre = Lv2Host::new().state_hash();
    let mut host = Lv2Host::new();
    host.set_firmware_identity("4.85", [0u8; 32]);
    assert_ne!(pre, host.state_hash());
}

#[test]
fn state_hash_differs_between_two_firmware_versions() {
    let mut a = Lv2Host::new();
    let mut b = Lv2Host::new();
    a.set_firmware_identity("4.85", [0u8; 32]);
    b.set_firmware_identity("4.86", [0u8; 32]);
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_equal_across_two_runs_of_same_firmware() {
    let mut a = Lv2Host::new();
    let mut b = Lv2Host::new();
    let digest: [u8; 32] = [0x42; 32];
    a.set_firmware_identity("4.85", digest);
    b.set_firmware_identity("4.85", digest);
    assert_eq!(a.state_hash(), b.state_hash());
}
