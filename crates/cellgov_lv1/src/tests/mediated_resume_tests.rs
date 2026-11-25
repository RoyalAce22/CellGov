use super::*;

/// lv1's handler ANDs each shadow doubleword with the mask doubleword and
/// skips the vector entry when no bit remains
/// [OFW-4.93 lv1 a:0x1A84-0x1AD4].
#[test]
fn a_take_after_the_mask_lost_its_bit_resumes_the_thread() {
    for mask in [0, !PLUG_BIT] {
        let mut hv = requested();
        assert_eq!(
            hv.take_mediated_external(&take(TAKE_PC, TAKE_MSR), &storage(mask)),
            Ok(MediatedEntry::Resume),
            "mask {mask:#x}"
        );
        assert_eq!(hv.refusal(), None, "mask {mask:#x}");
    }
}

/// The compare of SRR0 and the clear of the vector's MSR bits are in the
/// vector entry [OFW-4.93 lv1 a:0x188C-0x18A8], which the no-bit path never calls
/// [OFW-4.93 lv1 a:0x1AD4].
#[test]
fn a_take_with_no_shared_bit_resumes_at_the_trampoline_slot_and_with_msr_bit_56() {
    for (srr0, msr) in [(0x28B8, TAKE_MSR), (TAKE_PC, TAKE_MSR | 0x80)] {
        let mut hv = requested();
        assert_eq!(
            hv.take_mediated_external(&take(srr0, msr), &storage(0)),
            Ok(MediatedEntry::Resume),
            "srr0 {srr0:#x} msr {msr:#x}"
        );
    }
}

/// The handler's decrementer steps run before its shadow test
/// [OFW-4.93 lv1 a:0x1A04] [OFW-4.93 lv1 a:0x1A68].
#[test]
fn a_take_with_no_shared_bit_and_a_negative_decrementer_is_a_refusal() {
    let mut hv = requested();
    let negative = MediatedTake {
        decrementer: Some(0x8000_0000),
        ..take(TAKE_PC, TAKE_MSR)
    };
    let expected = refusal(
        TAKE_PC,
        "a take whose decrementer is negative, which lv1's handler resets and can send to its \
         trampoline slot 0x28B8",
    );
    assert_eq!(
        hv.take_mediated_external(&negative, &storage(0)),
        Err(expected)
    );
    assert_eq!(hv.refusal(), Some(expected));
}

#[test]
fn a_take_with_no_shared_bit_in_hypervisor_state_is_a_refusal() {
    let mut hv = requested();
    assert_eq!(
        hv.take_mediated_external(&take(TAKE_PC, TAKE_MSR | MSR_HV), &storage(0)),
        Err(refusal(
            TAKE_PC,
            "a take in hypervisor state, which lv1 treats as an interrupt of its own",
        ))
    );
}
