use super::*;

#[test]
fn new_boot_holds_exactly_the_boot_process() {
    let t = ProcessTable::new_boot();
    assert_eq!(t.len(), 1);
    let boot = t.boot();
    assert_eq!(boot.ppid, BOOT_PROCESS_PPID);
    assert_eq!(
        boot.authority_id,
        cellgov_ps3_abi::sce::RETAIL_APP_PROGRAM_AUTHORITY_ID
    );
    assert_eq!(boot.control_flags1, 0);
}

#[test]
fn boot_mut_edits_are_visible_through_get() {
    let mut t = ProcessTable::new_boot();
    t.boot_mut().authority_id = 0x1070_0005_ff00_0001;
    assert_eq!(
        t.get(BOOT_PROCESS_PID).unwrap().authority_id,
        0x1070_0005_ff00_0001
    );
}

#[test]
fn insert_child_refuses_existing_pid() {
    let mut t = ProcessTable::new_boot();
    let entry = ProcessEntry {
        ppid: BOOT_PROCESS_PID,
        authority_id: 0,
        control_flags1: 0,
        exit_status: None,
    };
    assert!(!t.insert_child(BOOT_PROCESS_PID, entry.clone()));
    assert!(t.insert_child(BOOT_PROCESS_PID + 1, entry));
    assert_eq!(t.len(), 2);
}

fn child_entry() -> ProcessEntry {
    ProcessEntry {
        ppid: BOOT_PROCESS_PID,
        authority_id: 0,
        control_flags1: 0,
        exit_status: None,
    }
}

#[test]
fn remove_child_never_removes_the_boot_entry() {
    let mut t = ProcessTable::new_boot();
    assert!(t.remove_child(BOOT_PROCESS_PID).is_none());
    assert_eq!(t.len(), 1);
}

#[test]
fn remove_child_drops_the_entry_and_only_its_unit_bindings() {
    use cellgov_event::UnitId;
    let mut t = ProcessTable::new_boot();
    let child = BOOT_PROCESS_PID + 0x100;
    let other = BOOT_PROCESS_PID + 0x200;
    assert!(t.insert_child(child, child_entry()));
    assert!(t.insert_child(other, child_entry()));
    t.bind_unit(UnitId::new(3), child);
    t.bind_unit(UnitId::new(5), child);
    t.bind_unit(UnitId::new(7), other);
    assert!(t.remove_child(child).is_some());
    assert!(t.get(child).is_none());
    assert!(t.units_of(child).is_empty());
    // A unit whose binding is gone reads as the boot process again.
    assert_eq!(t.process_of_unit(UnitId::new(3)), BOOT_PROCESS_PID);
    assert_eq!(t.units_of(other), vec![UnitId::new(7)]);
}

#[test]
fn remove_child_of_unknown_pid_returns_none() {
    let mut t = ProcessTable::new_boot();
    assert!(t.remove_child(BOOT_PROCESS_PID + 0x100).is_none());
}

#[test]
fn process_of_unit_defaults_to_boot() {
    let t = ProcessTable::new_boot();
    assert_eq!(
        t.process_of_unit(cellgov_event::UnitId::new(42)),
        BOOT_PROCESS_PID
    );
}

#[test]
fn units_of_yields_unit_id_order() {
    use cellgov_event::UnitId;
    let mut t = ProcessTable::new_boot();
    let child = BOOT_PROCESS_PID + 0x100;
    assert!(t.insert_child(child, child_entry()));
    t.bind_unit(UnitId::new(9), child);
    t.bind_unit(UnitId::new(2), child);
    assert_eq!(t.units_of(child), vec![UnitId::new(2), UnitId::new(9)]);
}

#[test]
fn next_child_pid_advances_from_the_highest_entry() {
    let mut t = ProcessTable::new_boot();
    assert_eq!(t.next_child_pid(), BOOT_PROCESS_PID + 0x100);
    assert!(t.insert_child(t.next_child_pid(), child_entry()));
    assert_eq!(t.next_child_pid(), BOOT_PROCESS_PID + 0x200);
}

#[test]
fn next_child_pid_saturates_and_then_collides_with_the_occupant() {
    let mut t = ProcessTable::new_boot();
    assert!(t.insert_child(u32::MAX - 0x80, child_entry()));
    assert_eq!(t.next_child_pid(), u32::MAX);
    assert!(t.insert_child(u32::MAX, child_entry()));
    // Saturated: the mint now names an occupied pid and insert_child
    // refuses it; dispatch must surface this instead of spawning.
    assert_eq!(t.next_child_pid(), u32::MAX);
    assert!(!t.insert_child(t.next_child_pid(), child_entry()));
}

#[test]
fn iter_yields_pid_order() {
    let mut t = ProcessTable::new_boot();
    t.insert_child(
        0x0100_0501,
        ProcessEntry {
            ppid: BOOT_PROCESS_PID,
            authority_id: 1,
            control_flags1: 0,
            exit_status: None,
        },
    );
    let pids: Vec<u32> = t.iter().map(|(pid, _)| *pid).collect();
    assert_eq!(pids, vec![BOOT_PROCESS_PID, 0x0100_0501]);
}
