//! Fresh module instances, the start phase they allow, and the live
//! instance the path lookup names after a withdrawal.

use super::*;

fn register_libadec(reg: &mut LoadedPrxRegistry) -> u32 {
    reg.register(
        "libadec".to_string(),
        "cellAdec".to_string(),
        0x0150_0000,
        0x0151_0000,
        0x0150_8000,
        Some(0x0150_1000),
        None,
    )
}

#[test]
fn register_instance_clones_the_image_under_a_new_initialized_id() {
    let mut reg = LoadedPrxRegistry::new();
    let first = register_libadec(&mut reg);
    reg.mark_started(first);
    let second = reg.register_instance(first).expect("first is registered");
    assert_eq!(second, first + 1);
    let entry = reg.lookup_by_id(second).unwrap();
    assert_eq!(entry.state(), PrxState::Initialized);
    assert_eq!(entry.base(), 0x0150_0000);
    assert_eq!(entry.stem(), "libadec");
    assert_eq!(
        reg.lookup_by_path("libadec.sprx").unwrap().kernel_id(),
        first,
        "the path keeps naming the instance it named"
    );
    assert_eq!(reg.register_instance(0xDEAD_BEEF), None);
}

#[test]
fn register_instance_refuses_once_the_kernel_ids_are_spent() {
    let mut reg = LoadedPrxRegistry::new();
    let first = register_libadec(&mut reg);
    reg.next_id = u32::MAX;
    assert_eq!(reg.register_instance(first), None);
    assert!(reg.lookup_by_id(u32::MAX).is_none());
}

#[test]
fn begin_start_moves_only_an_initialized_module() {
    let mut reg = LoadedPrxRegistry::new();
    let id = register_libadec(&mut reg);
    assert_eq!(reg.begin_start(id), Some(PrxState::Initialized));
    assert_eq!(reg.lookup_by_id(id).unwrap().state(), PrxState::Starting);
    assert_eq!(reg.begin_start(id), Some(PrxState::Starting));
    assert_eq!(reg.lookup_by_id(id).unwrap().state(), PrxState::Starting);
    assert!(reg.withdraw_removable(id).is_none(), "Starting refuses");
    assert_eq!(reg.begin_start(0xDEAD_BEEF), None);
}

#[test]
fn withdrawing_the_path_instance_moves_the_path_to_a_live_one() {
    let mut reg = LoadedPrxRegistry::new();
    let first = register_libadec(&mut reg);
    let second = reg.register_instance(first).unwrap();
    let third = reg.register_instance(first).unwrap();

    reg.withdraw(second);
    assert_eq!(
        reg.lookup_by_path("libadec.sprx").unwrap().kernel_id(),
        first,
        "withdrawing another instance leaves the path alone"
    );

    reg.withdraw(first);
    assert_eq!(
        reg.lookup_by_path("libadec.sprx").unwrap().kernel_id(),
        third
    );

    reg.withdraw(third);
    assert!(reg.lookup_by_path("libadec.sprx").is_none());
}
