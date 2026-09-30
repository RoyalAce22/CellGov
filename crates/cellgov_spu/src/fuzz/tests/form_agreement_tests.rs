//! The generator's encoding form for each kind agrees with the format the
//! opcode map records for that kind's row.

use super::*;
use cellgov_ps3_abi::hw::spu_isa::{row_named, SpuForm, SPU_OPCODE_MAP};

/// The generator form an ISA format maps to, or `None` for the hint format
/// only the control family uses.
fn generator_form(form: SpuForm) -> Option<SpuEncodingForm> {
    match form {
        SpuForm::Rr => Some(SpuEncodingForm::Rrr),
        SpuForm::Rrr => Some(SpuEncodingForm::Rrrr),
        SpuForm::Ri7 => Some(SpuEncodingForm::Ri7),
        SpuForm::Ri10 => Some(SpuEncodingForm::Ri10),
        SpuForm::Ri16 => Some(SpuEncodingForm::Ri16),
        SpuForm::Ri18 => Some(SpuEncodingForm::Ri18),
        SpuForm::Ri8 => Some(SpuEncodingForm::Ri8),
        SpuForm::Hint => None,
    }
}

/// [SPU-ISA p:28 s:2.3] RR, RRR and RI7; [SPU-ISA p:29 s:2.3] RI10, RI16 and RI18.
/// [SPU-ISA p:220 s:9] RI8 is not a basic format; the conversions place I8 after a 10-bit opcode.
#[test]
fn every_kind_has_the_form_its_opcode_row_records() {
    for descriptor in generation_descriptors() {
        let name: &'static str = descriptor.kind.into();
        let row = &SPU_OPCODE_MAP[row_named(&name.to_ascii_lowercase()).expect("a row")];
        // The families have independent signals: a branch transfers control and a
        // channel form draws channel numbers, so neither can hide in the RR default.
        assert_eq!(
            descriptor.sequence_flow == crate::fuzz::types::SpuSequenceFlow::ControlTransfer,
            descriptor.form == SpuEncodingForm::Branch,
            "{name}"
        );
        assert_eq!(
            !descriptor.channel_values.is_empty(),
            descriptor.form == SpuEncodingForm::Channel,
            "{name}"
        );
        match descriptor.form {
            // The families refine RR, RI16 and the hint format by what the word does.
            SpuEncodingForm::Branch | SpuEncodingForm::Channel | SpuEncodingForm::Control => {
                assert!(
                    matches!(row.form, SpuForm::Rr | SpuForm::Ri16 | SpuForm::Hint),
                    "{name} is a {:?} row",
                    row.form
                );
            }
            form => assert_eq!(
                Some(form),
                generator_form(row.form),
                "{name} is a {:?} row",
                row.form
            ),
        }
    }
}
