//! The 4.93 port id of the call that each thread's start makes with the
//! thread's record, held to the console capture of the two records.

use super::*;
use crate::lv1::boot_record_capture::captured_doubleword_in;
use crate::lv1::ppe::LOGICAL_PPE_RECORD_CAPTURE;

/// The kernel's per-thread records and the size of one record.
const RECORDS: u64 = 0x8000_0000_0036_8f80;
const RECORD_SIZE: u64 = 640;

/// The offset in a record where the kernel stores r4 of the call
/// [OFW-4.93 lv2_kernel a:0x8000000000027d44].
const PORT_OFFSET: u64 = 0x38;

/// The offset in a record of cntlzw of CTRL, the hardware thread that
/// CTRL names [OFW-4.93 lv2_kernel a:0x8000000000007d58].
const CTRL_OFFSET: u64 = 0x10;

/// The reads of the `lv2_logical_ppe_record` frame: three locator words,
/// the count, and words 0 to 12 of each record.
const RECORD_FRAME_READS: usize = 30;

/// The address and the value of the field `field` in the capture. Both
/// reads of the doubleword must agree.
fn captured(field: &str) -> (u64, u64) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(RECORD_PORT_CAPTURE)
        .join("peek/cech20-cex-493/record_ports.tsv");
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
fn each_thread_s_record_call_gets_the_id_at_0x38_of_its_captured_record() {
    assert_eq!(captured("toc").1, 0x8000_0000_0034_fbb0, "the TOC word");
    assert_eq!(captured("records").1, RECORDS, "the record array");
    for thread in 0u8..2 {
        let n = u64::from(thread);
        let record = RECORDS + RECORD_SIZE * n;
        assert_eq!(captured(&format!("index_{n}")), (record, n), "index");
        assert_eq!(
            captured(&format!("ctrl_{n}")),
            (record + CTRL_OFFSET, n),
            "the hardware thread of record {n}"
        );
        let (address, id) = captured(&format!("port_{n}"));
        assert_eq!(address, record + PORT_OFFSET, "the port row of record {n}");
        assert_eq!(
            game_os_port_id("4.93", thread, RECORD_START_SC),
            Some(id),
            "thread {thread}"
        );
    }
}

#[test]
fn the_record_probe_frame_reads_the_same_index_thread_and_port_words() {
    for n in 0..2 {
        for field in ["index", "ctrl", "port"] {
            let (address, value) = captured(&format!("{field}_{n}"));
            assert_eq!(
                captured_doubleword_in(LOGICAL_PPE_RECORD_CAPTURE, RECORD_FRAME_READS, address),
                value,
                "{field}_{n} at {address:#018x}"
            );
        }
    }
}
