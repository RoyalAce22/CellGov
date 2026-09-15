//! The local store is the private memory an SPU reports to the
//! explorer's observable.

use crate::SpuExecutionUnit;
use cellgov_event::UnitId;
use cellgov_exec::ExecutionUnit;

#[test]
fn an_spu_reports_its_local_store() {
    let unit = SpuExecutionUnit::new(UnitId::new(3));
    assert!(unit.local_memory_hash().is_some());
}

#[test]
fn one_local_store_byte_apart_is_a_different_hash() {
    let a = SpuExecutionUnit::new(UnitId::new(3));
    let mut b = SpuExecutionUnit::new(UnitId::new(3));
    b.state_mut().ls[0x1234] = 1;
    assert_ne!(a.local_memory_hash(), b.local_memory_hash());
}

#[test]
fn a_register_value_is_not_part_of_the_hash() {
    let a = SpuExecutionUnit::new(UnitId::new(3));
    let mut b = SpuExecutionUnit::new(UnitId::new(3));
    b.state_mut().set_reg_word_splat(7, 0xBEEF);
    assert_eq!(a.local_memory_hash(), b.local_memory_hash());
}
