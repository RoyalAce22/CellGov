//! `sys_process_get_sdk_version` held to the `elf_process_param` and
//! `elf_no_process_param` captures from the reference console: a process
//! whose record declares an SDK version reads that word back, and one
//! linked without a record reads the sentinel.

use super::*;
use cellgov_event::UnitId;

/// Word `i` of a probe's payload.
fn word(probe: &str, i: usize) -> u32 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
        .join("ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let at = 8 + 4 * i;
    u32::from_be_bytes(frame[at..at + 4].try_into().expect("4 bytes"))
}

/// The return code and the word CellGov's sc 25 writes.
fn answer(host: &Lv2Host) -> (u32, u32) {
    match host.dispatch_process_get_sdk_version(
        0x1000,
        UnitId::new(0),
        cellgov_time::GuestTicks::ZERO,
    ) {
        Lv2Dispatch::Immediate { code, effects } => {
            let [Effect::SharedWriteIntent { bytes, .. }] = effects.as_slice() else {
                panic!("one shared write, got {effects:?}");
            };
            let payload = bytes.bytes();
            (
                code as u32,
                u32::from_be_bytes(payload.try_into().expect("a u32")),
            )
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn a_process_with_a_record_reads_its_sdk_version_as_on_the_console() {
    let (record_sdk, rc, written) = (
        word("elf_process_param", 5),
        word("elf_process_param", 12),
        word("elf_process_param", 13),
    );
    let mut host = Lv2Host::new();
    host.set_sdk_version(record_sdk);
    assert_eq!(answer(&host), (rc, written));
}

#[test]
fn a_process_without_a_record_reads_the_sentinel_as_on_the_console() {
    assert_eq!(word("elf_no_process_param", 0), 0, "no record linked");
    let console = (
        word("elf_no_process_param", 12),
        word("elf_no_process_param", 13),
    );
    assert_eq!(answer(&Lv2Host::new()), console);
}
