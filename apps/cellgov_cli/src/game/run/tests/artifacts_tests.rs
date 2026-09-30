use super::*;

/// A unit stopped at `pc` whose local store holds `marker` at that address.
fn unit_at(pc: u32, marker: u8) -> SpuState {
    let mut state = SpuState::new();
    state.pc = pc;
    state.ls[pc as usize] = marker;
    state
}

#[test]
fn a_run_with_one_spu_unit_captures_it_without_a_unit_named() {
    let only = unit_at(0x200, 0xA1);
    let (id, capture) = capture_spu_unit(&[(4, &only)], None).expect("one unit");
    assert_eq!(id, 4);
    assert_eq!(capture, LocalStoreCapture::of(&only));
}

#[test]
fn a_named_unit_is_the_one_captured() {
    let (first, second) = (unit_at(0x200, 0xA1), unit_at(0x300, 0xB2));
    let spus = [(4, &first), (9, &second)];
    let (id, capture) = capture_spu_unit(&spus, Some(9)).expect("unit 9 is an SPU unit");
    assert_eq!(id, 9);
    assert_eq!((capture.pc, capture.local_store[0x300]), (0x300, 0xB2));
    assert!(matches!(
        capture_spu_unit(&spus, Some(2)),
        Err(RunError::NotAnSpuUnit { unit: 2, ref units }) if units == &[4, 9]
    ));
}

#[test]
fn a_run_with_no_spu_unit_or_several_unnamed_is_refused() {
    assert!(matches!(
        capture_spu_unit(&[], None),
        Err(RunError::NoSpuUnit)
    ));
    let (first, second) = (unit_at(0x200, 0xA1), unit_at(0x300, 0xB2));
    let several =
        capture_spu_unit(&[(4, &first), (9, &second)], None).expect_err("two units, none named");
    assert_eq!(
        several.to_string(),
        "save-spu-local-store: the run holds SPU units 4, 9; pass --spu-unit to pick one"
    );
}
