//! Each `SPU_Status` word an SPU thread stops with maps to one LV2
//! meaning.

use super::*;

/// The status word of a stop-and-signal with `code`.
fn stopped_with(code: u32) -> u32 {
    (code << 16) | 0b10
}

#[test]
fn the_exit_and_yield_codes_are_the_services_lv2_serves() {
    assert_eq!(
        SpuThreadStop::from_status(stopped_with(0x102)),
        SpuThreadStop::ThreadExit
    );
    assert_eq!(
        SpuThreadStop::from_status(stopped_with(0x101)),
        SpuThreadStop::GroupExit
    );
    assert_eq!(
        SpuThreadStop::from_status(stopped_with(0x100)),
        SpuThreadStop::Yield
    );
}

#[test]
fn any_other_stop_code_is_an_unserved_service() {
    for code in [0, 0x110, 0x111, 0x2100, 0x3FFF] {
        assert_eq!(
            SpuThreadStop::from_status(stopped_with(code)),
            SpuThreadStop::Error(SpuThreadError::UnservedStopCode(code as u16)),
            "code 0x{code:x}"
        );
    }
}

// [CBEA p:93 s:8.5.2] C is bit 25 and I bit 26.
// [CBEA p:94 s:8.5.2] H is bit 29.
#[test]
fn an_spu_error_and_a_halt_are_errors_named_by_their_status_bit() {
    assert_eq!(
        SpuThreadStop::from_status(1 << 5),
        SpuThreadStop::Error(SpuThreadError::InvalidInstruction)
    );
    assert_eq!(
        SpuThreadStop::from_status(1 << 6),
        SpuThreadStop::Error(SpuThreadError::InvalidChannel)
    );
    assert_eq!(
        SpuThreadStop::from_status(1 << 2),
        SpuThreadStop::Error(SpuThreadError::Halt)
    );
    assert_eq!(
        SpuThreadStop::from_status(0),
        SpuThreadStop::Error(SpuThreadError::NoCause(0))
    );
}
