//! `sys_process_get_number_of_object` answers each class from 0x00 to
//! 0xFF as the `lv2_process_classes` capture from the reference console
//! does: CELL_OK with a 4-byte count for a counted class, EINVAL with no
//! write for any other. The counts themselves are the console process's
//! resident objects at entry, which a fresh host does not hold.

use cellgov_event::UnitId;

use super::*;
use crate::host::test_support::{
    console_payload, extract_write_u32, seed_primary_ppu, FakeRuntime,
};
use crate::request::Lv2Request;
use crate::Lv2Dispatch;

const PROBE: &str = "lv2_process_classes";
const OFF_CLASSES: usize = 80;
const OUT_PTR: u32 = 0x100;

fn word(payload: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(payload[at..at + 8].try_into().expect("8 bytes"))
}

#[test]
fn every_object_class_answers_as_on_the_console() {
    let payload = console_payload(PROBE);
    assert_eq!(payload.len(), OFF_CLASSES + 256 * 16);
    let rt = FakeRuntime::new(0x1000);
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, UnitId::new(0));

    let mut counted = 0;
    let mut differing = Vec::new();
    for class in 0..=0xFFu32 {
        let at = OFF_CLASSES + class as usize * 16;
        let console_rc = word(&payload, at);
        // The call writes a 4-byte count over the top half of the
        // probe's 0xEE-filled word, or nothing.
        let console_wrote = word(&payload, at + 8) >> 32 != 0xEEEE_EEEE;
        counted += usize::from(console_wrote);
        let (code, wrote) = match host.dispatch(
            Lv2Request::ProcessGetNumberOfObject {
                class_id: class,
                count_out_ptr: OUT_PTR,
            },
            UnitId::new(0),
            &rt,
        ) {
            Lv2Dispatch::Immediate { code, effects } => {
                (code, effects.first().map(extract_write_u32).is_some())
            }
            other => panic!("class {class:#x}: {other:?}"),
        };
        if (code, wrote) != (console_rc, console_wrote) {
            differing.push(format!(
                "class {class:#04x}: cellgov ({code:#x}, {wrote}), console ({console_rc:#x}, {console_wrote})"
            ));
        }
    }
    assert_eq!(counted, 24, "the console counts 24 classes");
    assert!(
        differing.is_empty(),
        "(rc, count written) differs:\n{}",
        differing.join("\n")
    );
}
