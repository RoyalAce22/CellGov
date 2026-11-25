use super::*;

/// The PPU takes the mediated external interrupt before the decrementer
/// interrupt [CBE-Handbook p:291 s:9.10]. lv1's handler then finds the
/// decrementer negative [OFW-4.93 lv1 a:0x2C632C-0x2C6334].
#[test]
fn a_take_with_a_negative_decrementer_stops_the_run() {
    let (kernel, after_move) = requesting_kernel(After::NegativeDecrementer);
    let run = run_kernel_from_entry(&kernel, FW_MEDIATED, 2_000).unwrap();
    assert_eq!(
        take_refusal(&run),
        (
            after_move,
            "a take whose decrementer is negative, which lv1's handler resets and can send to \
             its trampoline slot 0x28B8"
        )
    );
}
