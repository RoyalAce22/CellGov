//! The SPU has no special-purpose registers: `mfspr` reads zero and
//! `mtspr` changes nothing, for every SA.

use super::*;
use crate::decode::decode;
use crate::state::{SpuObservableSnapshot, SpuState};

/// SA values across the 7-bit field, both ends included.
const SAS: [u32; 4] = [0, 1, 64, 127];

/// [SPU-ISA p:244 s:10] an undefined SPR supplies zeros; [CBE-Handbook p:67 s:3.1.2] the SPU has no SPRs.
#[test]
fn mfspr_writes_128_zero_bits_for_every_sa() {
    for sa in SAS {
        let insn = decode((0x00C << 21) | (sa << 7) | 5).expect("mfspr decodes");
        assert_eq!(
            insn,
            SpuInstruction::Mfspr {
                rt: 5,
                sa: sa as u8
            }
        );
        let mut s = SpuState::new();
        s.regs[5] = [0xA5; 16];
        let out = execute(&insn, &mut s, UnitId::new(0));
        assert_eq!(out, SpuStepOutcome::Continue);
        assert_eq!(s.regs[5], [0; 16], "sa {sa}");
    }
}

/// [SPU-ISA p:245 s:10] writing an undefined SPR performs no operation.
#[test]
fn mtspr_changes_no_state_for_any_sa() {
    for sa in SAS {
        let insn = decode((0x10C << 21) | (sa << 7) | 5).expect("mtspr decodes");
        assert_eq!(
            insn,
            SpuInstruction::Mtspr {
                sa: sa as u8,
                rt: 5
            }
        );
        let mut s = SpuState::new();
        s.regs[5] = [0xA5; 16];
        let before = SpuObservableSnapshot::capture(&s);
        let out = execute(&insn, &mut s, UnitId::new(0));
        assert_eq!(out, SpuStepOutcome::Continue);
        assert_eq!(SpuObservableSnapshot::capture(&s), before, "sa {sa}");
    }
}
