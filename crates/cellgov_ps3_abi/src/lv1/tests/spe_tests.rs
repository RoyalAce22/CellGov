use super::*;
use crate::lv1::call::{self, CallSource, CONSTRUCT_LOGICAL_SPE};
use crate::lv1::memory::{region_address, REGION_SIZE_EXP_SHIFT};
use crate::lv1::table::{table_entry, TableEntry};

/// The shadow page size of the kernel's boot call, whose r7 is 12
/// [OFW-4.93 lv2_kernel a:0x800000000004f168].
const KERNEL_SHADOW_BYTES: u64 = 0x1000;

/// The value of the field `field` in the capture file. Both reads of the
/// word must agree.
fn captured(field: &str) -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(LOGICAL_SPE_OBJECT_CAPTURE)
        .join("peek/cech20-cex-493/spe_object.tsv");
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("address\tfield\twidth\treads\tagree\tvalue"),
        "{name}: header"
    );
    let row: Vec<&str> = lines
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .find(|row| row.get(1) == Some(&field))
        .unwrap_or_else(|| panic!("{name}: no row for {field}"));
    assert_eq!(
        (row[2], row[3], row[4]),
        ("8", "2", "yes"),
        "{name}: {field}"
    );
    u64::from_str_radix(row[5].trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("{name}: {field}: {e}"))
}

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 57.
#[test]
fn construct_logical_spe_carries_its_header_number_name_and_counts() {
    assert_eq!(CONSTRUCT_LOGICAL_SPE, 57);
    let row = call::lookup(CONSTRUCT_LOGICAL_SPE).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some(("construct_logical_spe", 7, 6, CallSource::Linux))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_57() {
    let handler = construct_logical_spe_handler("4.93").expect("the 4.93 handler");
    assert_eq!(
        table_entry("4.93", CONSTRUCT_LOGICAL_SPE),
        Some(TableEntry::Handler(handler.handler))
    );
    assert_eq!(construct_logical_spe_handler("4.92"), None);
}

/// The stock TOC slot holds the object's address
/// [OFW-4.93 lv2_kernel a:0x8000000000348ae8].
#[test]
fn the_kernels_boot_call_has_the_captured_spe_id() {
    assert_eq!(
        captured("object"),
        0x8000_0000_003d_df48,
        "the TOC slot holds its stock value"
    );
    let call = game_os_spe_call("4.93", 0x8000_0000_0004_f188).expect("the boot call");
    assert_eq!(call.spe_type, LOGICAL_SPE_TYPE_1);
    assert_eq!(call.id, captured("spe_id"));
}

#[test]
fn the_captured_shadow_is_the_4_kb_region_of_the_value_after_the_areas() {
    let shadow = captured("shadow");
    assert_eq!(shadow >> REGION_SIZE_EXP_SHIFT, 12, "the size exponent");
    let counter = (shadow & ((1 << REGION_SIZE_EXP_SHIFT) - 1)) >> 12;
    assert_eq!(region_address(KERNEL_SHADOW_BYTES, counter), Some(shadow));
    assert!(counter > 0, "the area takes the value before the shadow's");
    let area = region_address(LOGICAL_SPE_AREA_SIZE, counter - 1).expect("the area");
    assert_eq!(area >> REGION_SIZE_EXP_SHIFT, 19, "a 512 KB region");
}

#[test]
fn each_call_with_an_spe_id_passes_the_type_whose_worker_is_read() {
    let calls: Vec<&GameOsSpeCall> = GAME_OS_SPE_CALLS
        .iter()
        .flat_map(|calls| calls.calls)
        .collect();
    assert!(!calls.is_empty());
    for call in calls {
        assert_eq!(call.spe_type, LOGICAL_SPE_TYPE_1, "0x{:016x}", call.sc);
    }
}

/// The kernel's type 0 call [OFW-4.93 lv2_kernel a:0x80000000000356dc]
/// and type 2 call [OFW-4.93 lv2_kernel a:0x8000000000036358].
#[test]
fn only_the_captured_call_has_an_spe_id() {
    let uncaptured = [
        ("4.93", 0x8000_0000_0003_56dc),
        ("4.93", 0x8000_0000_0003_6358),
        ("4.92", 0x8000_0000_0004_f188),
    ];
    for (firmware, sc) in uncaptured {
        assert_eq!(
            game_os_spe_call(firmware, sc),
            None,
            "{firmware} 0x{sc:016x}"
        );
    }
}
