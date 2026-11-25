//! The hypervisor answers the 4.93 kernel's reads of devices 1 to 3 of
//! bus 1 after the open of device 1 as lv1 does on the reference console,
//! from the committed capture `tests/micro/lv1_repository_bus`.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::repository::{field, RepositoryKey};

use super::console_bus_device_tests::{answered, device, dispatch_4_93, read};
use crate::{HypercallAnswer, HypercallDisposition, HypercallRefusal, Hypervisor};

fn region(index: u64, word: &[u8]) -> [u64; 4] {
    device(1, [field(b"reg", index), field(word, 0)])
}

fn interrupt(index: u64) -> [u64; 4] {
    device(1, [field(b"intr", index), 0])
}

/// lv1's answer when the walk finds no node: r3 = 0x0000001EFFFFFFFA
/// [OFW-4.93 lv1 a:0x2E3E9C-0x2E3EB8], with the values that the handler
/// cleared before the lookup [OFW-4.93 lv1 a:0x2E52E0-0x2E5300].
fn no_node() -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(0x0000_001E_FFFF_FFFA, [0; 7]))
}

/// The capture's `node.tsv` holds type 3 at 0x3010000 with size 0x10000
/// for region 0, and type 4 at 0x3810000 with size 0x10000 for region 1.
/// lv1 returns status 0 with v1 and v2 [OFW-4.93 lv1 a:0x2E53AC-0x2E53B8].
#[test]
fn the_region_reads_of_device_1_answer_status_0_with_the_captured_type_address_and_size() {
    let cases = [
        (region(0, b"type"), (0, 3, 0)),
        (region(0, b"data"), (0, 0x0301_0000, 0x0001_0000)),
        (region(1, b"type"), (0, 4, 0)),
        (region(1, b"data"), (0, 0x0381_0000, 0x0001_0000)),
    ];
    assert_eq!(cases.len(), 4);
    for (n, expected) in cases {
        assert_eq!(answered(n), expected, "{n:x?}");
    }
}

/// The capture's `node.tsv` holds v1 = 3 and v2 = 0x10 for interrupt 0,
/// and v1 = 4 and v2 = 0xA for interrupt 1.
#[test]
fn the_interrupt_reads_of_device_1_answer_status_0_with_the_captured_values() {
    assert_eq!(answered(interrupt(0)), (0, 3, 0x10));
    assert_eq!(answered(interrupt(1)), (0, 4, 0x0a));
}

/// The capture's walk covered the whole list and holds no region 2 and no
/// interrupt 2 of device 1. The kernel's loops end at those reads
/// [OFW-4.93 lv2_kernel a:0x8000000000093550-0x800000000009356c]
/// [OFW-4.93 lv2_kernel a:0x80000000000935cc-0x80000000000935d4].
#[test]
fn the_reads_past_the_last_region_and_interrupt_of_device_1_answer_lv1s_no_node_status() {
    assert_eq!(dispatch_4_93(&read(region(2, b"type"))), no_node());
    assert_eq!(dispatch_4_93(&read(interrupt(2))), no_node());
}

/// The capture's `node.tsv` holds the regions and interrupts of device 2,
/// and no region 2 or interrupt 2 of it. The handler of type 4 reads them
/// as it reads those of device 1.
#[test]
fn the_region_and_interrupt_reads_of_device_2_answer_the_capture_and_lv1s_no_node_status() {
    let of_2 = |n: [u64; 4]| [n[0], field(b"dev", 2), n[2], n[3]];
    let cases = [
        (of_2(region(0, b"type")), (0, 3, 0)),
        (of_2(region(0, b"data")), (0, 0x0302_0000, 0x0001_0000)),
        (of_2(region(1, b"type")), (0, 4, 0)),
        (of_2(region(1, b"data")), (0, 0x0382_0000, 0x0001_0000)),
        (of_2(interrupt(0)), (0, 3, 0x11)),
        (of_2(interrupt(1)), (0, 4, 0x0b)),
    ];
    assert_eq!(cases.len(), 6);
    for (n, expected) in cases {
        assert_eq!(answered(n), expected, "{n:x?}");
    }
    assert_eq!(dispatch_4_93(&read(of_2(region(2, b"type")))), no_node());
    assert_eq!(dispatch_4_93(&read(of_2(interrupt(2)))), no_node());
}

/// The capture's `node.tsv` holds type 6, id 3 and interrupt 0 of device
/// 3, and no region of it. The handler of type 6 reads the id, region 0's
/// type and interrupt 0 [OFW-4.93 lv2_kernel a:0x8000000000093318-0x80000000000933ac].
#[test]
fn the_reads_of_device_3_answer_the_capture_and_lv1s_no_node_status_for_region_0() {
    assert_eq!(answered(device(3, [field(b"type", 0), 0])), (0, 6, 0));
    assert_eq!(answered(device(3, [field(b"id", 0), 0])), (0, 3, 0));
    assert_eq!(answered(device(3, [field(b"intr", 0), 0])), (0, 5, 0x2f));
    assert_eq!(
        dispatch_4_93(&read(device(3, [field(b"reg", 0), field(b"type", 0)]))),
        no_node()
    );
}

/// No walk at another firmware shows the key absent, and the absent-key
/// table holds only the absent keys that a logged boot reads.
#[test]
fn an_absent_key_at_another_firmware_or_one_no_boot_reads_is_a_refusal() {
    let mut floor = Hypervisor::new();
    floor.set_firmware("3.70");
    let past = region(3, b"type");
    let cases = [
        (
            floor.dispatch(&read(region(2, b"type"))),
            region(2, b"type"),
        ),
        (dispatch_4_93(&read(past)), past),
        (dispatch_4_93(&read(region(2, b"data"))), region(2, b"data")),
    ];
    assert_eq!(cases.len(), 3);
    for (disposition, n) in cases {
        assert_eq!(
            disposition,
            HypercallDisposition::Refused(HypercallRefusal::RepositoryNode {
                key: RepositoryKey {
                    lpar: 1,
                    n1: n[0],
                    n2: n[1],
                    n3: n[2],
                    n4: n[3],
                },
                unit: UnitId::new(0),
                pc: 0x8000_0000_0009_22fc,
            }),
            "{n:x?}"
        );
    }
}
