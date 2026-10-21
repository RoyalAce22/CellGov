//! The unit that runs a thread while a module_start unit holds an alias
//! to it.

use super::*;

fn attrs() -> PpuThreadAttrs {
    PpuThreadAttrs {
        entry: 0,
        arg: 0,
        stack_base: 0,
        stack_size: 0,
        priority: 0,
        tls_base: 0,
    }
}

#[test]
fn a_thread_runs_on_its_live_alias_and_on_its_own_unit_once_the_alias_drops() {
    let mut t = PpuThreadTable::new();
    t.insert_primary(UnitId::new(0), attrs());
    assert_eq!(t.running_unit(PpuThreadId::PRIMARY), Some(UnitId::new(0)));
    assert!(t.alias_unit(UnitId::new(7), PpuThreadId::PRIMARY));
    assert_eq!(t.running_unit(PpuThreadId::PRIMARY), Some(UnitId::new(7)));
    assert!(t.alias_unit(UnitId::new(9), PpuThreadId::PRIMARY));
    assert_eq!(
        t.running_unit(PpuThreadId::PRIMARY),
        Some(UnitId::new(9)),
        "the newest alias runs the thread",
    );
    assert!(t.drop_alias(UnitId::new(9)));
    assert!(t.drop_alias(UnitId::new(7)));
    assert_eq!(t.running_unit(PpuThreadId::PRIMARY), Some(UnitId::new(0)));
    assert_eq!(t.running_unit(PpuThreadId::new(0x0100_0099)), None);
}
