//! A refused word between `li r11, N` and `sc` drops the candidate.

use super::*;

/// `li r11, 22`.
const LI_R11_22: u32 = (14 << 26) | (11 << 21) | 22;
/// `sc`.
const SC: u32 = 0x4400_0002;
/// `lq r10, 0(r3)`: RT+1 is r11.
const LQ_R10: u32 = (56 << 26) | (10 << 21) | (3 << 16);
/// `tlbia`, which names no register.
const TLBIA: u32 = (31 << 26) | (370 << 1);

#[test]
fn a_refused_word_after_the_r11_constant_drops_the_candidate() {
    assert!(matches!(
        crate::decode::decode(LQ_R10),
        Ok(PpuInstruction::PpePrivileged { .. })
    ));
    assert!(matches!(
        crate::decode::decode(TLBIA),
        Ok(PpuInstruction::PpeIllegal { .. })
    ));
    for refused in [LQ_R10, TLBIA] {
        let sites = scan_words(0, &[LI_R11_22, refused, SC]);
        assert_eq!(sites.len(), 1, "0x{refused:08x}");
        assert_eq!(sites[0].ordinal, None, "0x{refused:08x}");
    }
}
