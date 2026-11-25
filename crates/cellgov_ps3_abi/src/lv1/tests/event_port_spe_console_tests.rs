//! The 4.93 port id of the call in the kernel's create function of its
//! boot logical SPE, held to the console capture of the SPE object.

use super::*;
use crate::lv1::spe::LOGICAL_SPE_OBJECT_CAPTURE;

/// The offset in the SPE object where the kernel stores r4 of the call
/// [OFW-4.93 lv2_kernel a:0x800000000004f224].
const PORT_OFFSET: u64 = 88;

/// The address and the value of the field `field` in the capture. Both
/// reads of the doubleword must agree.
fn captured(field: &str) -> (u64, u64) {
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
    assert_eq!(row.len(), 6, "{name}: {field} columns");
    assert_eq!(
        (row[2], row[3], row[4]),
        ("8", "2", "yes"),
        "{name}: {field}"
    );
    let hex = |text: &str| {
        u64::from_str_radix(text.trim_start_matches("0x"), 16)
            .unwrap_or_else(|e| panic!("{name}: {field}: {e}"))
    };
    (hex(row[0]), hex(row[5]))
}

#[test]
fn the_boot_spe_call_of_thread_0_gets_the_id_at_88_of_the_captured_object() {
    let (_, object) = captured("object");
    assert_eq!(object, 0x8000_0000_003d_df48, "the TOC slot's stock value");
    let (address, id) = captured("port");
    assert_eq!(address, object + PORT_OFFSET, "the port row's address");
    assert_eq!(game_os_port_id("4.93", 0, SPE_SC), Some(id));
}
