use super::*;

#[test]
fn the_ppe_ignores_writes_to_ile_at_bit_47_and_le_at_bit_63() {
    // [CBE-Handbook p:304 s:10.4.1] The PPE's MSR holds ILE at bit 47 and LE at bit 63.
    assert_eq!(MSR_ILE, 0x0000_0000_0001_0000);
    assert_eq!(MSR_LE, 0x0000_0000_0000_0001);
    // [CBE-Handbook p:739 s:A.2.5] The PPE supports only big-endian mode, and a write to MSR[LE] or MSR[ILE] has no effect.
    assert_eq!(PPE_MSR_IGNORED, 0x0000_0000_0001_0001);
}
