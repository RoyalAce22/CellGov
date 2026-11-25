use super::tests::captured;
use super::*;
use crate::lv1::call::{self, CallSource, GET_SPE_IRQ_OUTLET};
use crate::lv1::table::{table_entry, TableEntry};

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 78.
#[test]
fn get_spe_irq_outlet_carries_its_header_number_name_and_counts() {
    assert_eq!(GET_SPE_IRQ_OUTLET, 78);
    let row = call::lookup(GET_SPE_IRQ_OUTLET).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(row, Some(("get_spe_irq_outlet", 2, 1, CallSource::Linux)));
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_78() {
    let handler = spe_irq_outlet_handler("4.93").expect("the 4.93 handler");
    assert_eq!(
        table_entry("4.93", GET_SPE_IRQ_OUTLET),
        Some(TableEntry::Handler(handler.handler))
    );
    assert_eq!(spe_irq_outlet_handler("4.92"), None);
}

#[test]
fn the_kernels_boot_spe_has_the_captured_outlet_ids() {
    let call = game_os_spe_call("4.93", 0x8000_0000_0004_f188).expect("the boot call");
    assert_eq!(
        call.outlets,
        [
            captured("outlet_0"),
            captured("outlet_1"),
            captured("outlet_2")
        ]
    );
}

/// The type 1 worker constructs the SPE object, which takes its id
/// [OFW-4.93 lv1 a:0x315948], before it makes the outlets one class after
/// another [OFW-4.93 lv1 a:0x315A38-0x315AE8]. Each takes the first free
/// id of the object table, so the ids rise in that order.
#[test]
fn each_spes_outlet_ids_rise_in_class_order_above_its_spe_id() {
    let calls: Vec<&GameOsSpeCall> = GAME_OS_SPE_CALLS
        .iter()
        .flat_map(|calls| calls.calls)
        .collect();
    assert!(!calls.is_empty());
    for call in calls {
        let mut previous = call.id;
        for outlet in call.outlets {
            assert!(outlet > previous, "0x{:016x}: {:?}", call.sc, call.outlets);
            previous = outlet;
        }
    }
}

#[test]
fn the_boot_call_is_found_by_its_spe_id_alone() {
    let call = game_os_spe_call("4.93", 0x8000_0000_0004_f188).expect("the boot call");
    assert_eq!(game_os_spe_by_id("4.93", call.id), Some(call));
    assert_eq!(game_os_spe_by_id("4.93", call.id + 1), None);
    assert_eq!(game_os_spe_by_id("4.92", call.id), None);
}
