//! `sc` at and past the end of the syscall table, held to the
//! `lv2_syscall_bound` capture from the reference console.

use super::*;
use crate::host::test_support::console_payload;
use crate::request::classify;
use cellgov_ps3_abi::lv2::syscall::UNRESOLVED_IMPORT;

/// The probe's r11 values, in payload order.
const R11: [u64; 9] = [
    0,
    1023,
    1024,
    1025,
    0xFFFF,
    0x10000,
    0x7FFFF,
    0x80000,
    0x1_0000_0001,
];

#[test]
fn sc_past_the_table_answers_enosys_as_the_console_does() {
    let payload = console_payload("lv2_syscall_bound");
    assert_eq!(payload.len(), 8 * R11.len());
    for (i, &r11) in R11.iter().enumerate() {
        let console = u64::from_be_bytes(payload[8 * i..8 * i + 8].try_into().expect("8 bytes"));
        assert_eq!(
            console as u32,
            errno::CELL_ENOSYS.code,
            "r11 {r11:#x} on the console"
        );
        // The console sign-extends the code into r3; CellGov compares
        // the low word here.
        assert_eq!(console >> 32, 0xFFFF_FFFF, "r11 {r11:#x}: r3 sign-extended");
        let request = classify(r11, &[0; 8]);
        if r11 == UNRESOLVED_IMPORT {
            // CellGov keeps this number for its import trampoline.
            assert!(
                matches!(request, Lv2Request::UnresolvedImport { .. }),
                "{request:?}"
            );
            continue;
        }
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let Lv2Dispatch::Immediate { code, .. } = host.dispatch(request, UnitId::new(0), &rt)
        else {
            panic!("r11 {r11:#x}: expected an immediate answer");
        };
        assert_eq!(code as u32, console as u32, "r11 {r11:#x}");
    }
}
