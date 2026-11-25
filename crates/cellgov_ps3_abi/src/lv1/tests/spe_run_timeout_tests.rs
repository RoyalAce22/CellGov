use super::*;
use crate::lv1::call::{self, Alias, AliasSource, CallSource, LOGICAL_SPU_SET_RUN_TIMEOUT};
use crate::lv1::table::{table_entry, TableEntry};

/// The row is the `undocumented_function_138` row of the OtherOS++ copy
/// of `lv1call.h`, with CellGov's name from the handler.
#[test]
fn logical_spu_set_run_timeout_carries_its_header_row_and_cellgov_name() {
    assert_eq!(LOGICAL_SPU_SET_RUN_TIMEOUT, 138);
    let row = call::lookup(LOGICAL_SPU_SET_RUN_TIMEOUT)
        .map(|c| (c.name, c.inputs, c.outputs, c.source, c.alias));
    assert_eq!(
        row,
        Some((
            "undocumented_function_138",
            2,
            0,
            CallSource::OtherOsPlusPlus,
            Some(Alias {
                name: "logical_spu_set_run_timeout",
                source: AliasSource::CellGov,
            }),
        ))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_138() {
    let handler = spe_run_timeout_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x31BD54);
    assert_eq!(
        table_entry("4.93", LOGICAL_SPU_SET_RUN_TIMEOUT),
        Some(TableEntry::Handler(handler.handler))
    );
    assert_eq!(spe_run_timeout_handler("4.92"), None);
}
