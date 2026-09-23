//! Generated-input properties of `decode` over the 32-bit word space.

use proptest::prelude::*;

use super::decode;
use crate::instruction::encode::{alias, encode, reserved_bits, Alias};
use crate::instruction::{PpuDecodeError, PpuInstruction};

/// Words weighted toward the primaries whose extended opcode decides
/// the decode; a uniform word is a D-form instruction about half the
/// time.
fn instruction_words() -> impl Strategy<Value = u32> {
    let extended = prop_oneof![Just(4u32), Just(19), Just(30), Just(31), Just(59), Just(63)];
    prop_oneof![
        2 => any::<u32>(),
        3 => (extended, any::<u32>()).prop_map(|(primary, rest)| (primary << 26) | (rest & 0x03FF_FFFF)),
    ]
}

proptest! {
    #[test]
    fn decode_classifies_every_word_without_a_panic(raw in any::<u32>()) {
        match decode(raw) {
            Ok(_) => {}
            Err(PpuDecodeError::DecoderArmUnimplemented { raw: rejected, .. })
            | Err(PpuDecodeError::EncodingNotRecognized { raw: rejected }) => {
                prop_assert_eq!(rejected, raw);
            }
        }
    }

    #[test]
    fn a_decoded_word_re_encodes_to_its_canonical_form(raw in instruction_words()) {
        let Ok(insn) = decode(raw) else { return Ok(()) };
        let encoded = match encode(&insn) {
            Ok(word) => word,
            Err(e) => return Err(TestCaseError::fail(format!("{raw:#010x} {insn:?}: {e}"))),
        };
        prop_assert_eq!(decode(encoded), Ok(insn));
        match alias(raw) {
            None => prop_assert_eq!(encoded, raw & !reserved_bits(&insn)),
            Some(Alias::NopHint { .. }) => prop_assert_eq!(
                insn,
                PpuInstruction::Ori { ra: 0, rs: 0, imm: 0 }
            ),
            Some(Alias::TimeBaseThroughMfspr { tbr }) => {
                let is_time_base_read = matches!(
                    (tbr, insn),
                    (268, PpuInstruction::Mftb { .. }) | (269, PpuInstruction::Mftbu { .. })
                );
                prop_assert!(is_time_base_read, "tbr {} decoded as {:?}", tbr, insn);
            }
        }
    }
}
