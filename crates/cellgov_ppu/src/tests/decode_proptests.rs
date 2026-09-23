//! Generated-input properties of `decode` over the 32-bit word space.

use proptest::prelude::*;

use super::decode;
use crate::instruction::PpuDecodeError;

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
}
