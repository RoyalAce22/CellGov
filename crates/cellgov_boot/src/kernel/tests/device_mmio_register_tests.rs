//! The register words that a kernel run declares in the MMIO spans of
//! the system bus devices.

use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::device_register::DEVICE_REGISTER_READS;
use cellgov_ps3_abi::lv1::device_bus::MMIO_REAL_BASE;

use super::declare_device_mmio;

fn declared(firmware: &str) -> GuestMemory {
    let mut mem = GuestMemory::new(0x1000);
    declare_device_mmio(&mut mem, firmware).unwrap();
    mem
}

#[test]
fn each_4_93_register_read_answers_its_reader_only() {
    let mem = declared("4.93");
    let reads: Vec<_> = DEVICE_REGISTER_READS
        .iter()
        .filter(|r| r.firmware == "4.93")
        .collect();
    assert_eq!(reads.len(), 2);
    for read in reads {
        let real = MMIO_REAL_BASE + read.region + read.offset;
        let word = read.value.to_be_bytes();
        assert_eq!(
            mem.sourced_word(real, read.reader),
            Some((real, &word[..])),
            "{read:?}"
        );
        assert_eq!(mem.sourced_word(real, read.reader + 4), None, "{read:?}");
        assert_eq!(mem.sourced_word(real + 8, read.reader), None, "{read:?}");
    }
}

#[test]
fn a_firmware_with_no_device_regions_declares_no_register_word() {
    let mem = declared("4.92");
    assert!(mem.unmodelled_spans().is_empty());
    assert_eq!(DEVICE_REGISTER_READS.len(), 2);
    for read in DEVICE_REGISTER_READS {
        let real = MMIO_REAL_BASE + read.region + read.offset;
        assert_eq!(mem.sourced_word(real, read.reader), None, "{read:?}");
    }
}
