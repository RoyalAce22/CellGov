use super::*;

const NEGATIVE_REFUSAL: &str = "a take whose decrementer is negative, which lv1's handler resets \
                                and can send to its trampoline slot 0x28B8";

fn take_with(decrementer: Option<u32>) -> MediatedTake {
    MediatedTake {
        decrementer,
        ..take(TAKE_PC, TAKE_MSR)
    }
}

/// lv1's handler tests the decrementer with a signed word compare
/// [OFW-4.93 lv1 a:0x2C632C-0x2C6334].
#[test]
fn a_take_whose_decrementer_is_negative_is_a_refusal() {
    for decrementer in [0x8000_0000, 0xFFFF_FFFF] {
        let mut hv = requested();
        assert_eq!(
            hv.take_mediated_external(&take_with(Some(decrementer)), &storage(PLUG_BIT)),
            Err(refusal(TAKE_PC, NEGATIVE_REFUSAL)),
            "decrementer {decrementer:#x}"
        );
    }
}

/// The compare branches only on less than zero, so a decrementer of 0 is
/// not negative [OFW-4.93 lv1 a:0x2C632C-0x2C6334].
#[test]
fn a_take_whose_decrementer_is_0_or_0x7fffffff_is_answered() {
    for decrementer in [0, 0x7FFF_FFFF] {
        let mut hv = requested();
        assert_eq!(
            hv.take_mediated_external(&take_with(Some(decrementer)), &storage(PLUG_BIT)),
            Ok(()),
            "decrementer {decrementer:#x}"
        );
    }
}

#[test]
fn a_take_on_a_unit_with_no_decrementer_is_a_refusal() {
    let mut hv = requested();
    assert_eq!(
        hv.take_mediated_external(&take_with(None), &storage(PLUG_BIT)),
        Err(refusal(
            TAKE_PC,
            "a take on a unit that gives no decrementer"
        ))
    );
}
