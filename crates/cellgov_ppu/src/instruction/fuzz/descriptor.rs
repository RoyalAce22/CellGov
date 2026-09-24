//! Encoding, packing, aliasing and shrinking through a generation descriptor.

use cellgov_exec::operand::{self, OperandPackError};

use crate::instruction::PpuInstructionKind;

use super::classify::exact_kind;
use super::fields::{discriminator_bits, generation_operands_are_valid, semantic_reserved_bits};
use super::types::{PpuFuzzKind, PpuGenerationDescriptor, PpuGenerationError};

impl PpuGenerationDescriptor {
    /// Encodes one value per typed operand field.
    ///
    /// # Errors
    ///
    /// - If the number of values is incorrect, the method returns [`PpuGenerationError::OperandCount`].
    /// - If the operands are invalid, the method returns [`PpuGenerationError::InvalidOperands`].
    pub fn encode(&self, values: &[u32]) -> Result<u32, PpuGenerationError> {
        let word = self.pack_operands(values)?;
        let instruction =
            crate::decode::decode(word).map_err(|_| PpuGenerationError::InvalidOperands)?;
        (instruction.fuzz_descriptor(word).kind == self.kind
            && generation_operands_are_valid(instruction))
        .then_some(word)
        .ok_or(PpuGenerationError::InvalidOperands)
    }

    /// Packs in-range descriptor operands without consulting the decoder.
    pub fn pack_operands(&self, values: &[u32]) -> Result<u32, PpuGenerationError> {
        operand::pack_operands(self.canonical_word, &self.operands, values).map_err(|error| {
            match error {
                OperandPackError::Count { expected, found } => {
                    PpuGenerationError::OperandCount { expected, found }
                }
                OperandPackError::OutOfRange => PpuGenerationError::InvalidOperands,
            }
        })
    }

    /// Checks documented operand restrictions using raw fields, independent of decoding.
    pub fn operands_are_defined(&self, word: u32) -> bool {
        use PpuInstructionKind as K;
        let rt = (word >> 21) & 31;
        let ra = (word >> 16) & 31;
        let rb = (word >> 11) & 31;
        match self.kind {
            // [PPC-Book1 p:33 s:3.3.2] Fixed-point load-update requires a nonzero RA distinct from RT.
            PpuFuzzKind::Ordinary(
                K::Lhau
                | K::Lwzu
                | K::Lbzu
                | K::Lhzu
                | K::Ldu
                | K::Lwzux
                | K::Lbzux
                | K::Lhzux
                | K::Ldux
                | K::Lhaux
                | K::Lwaux,
            ) => ra != 0 && ra != rt,
            // [PPC-Book1 p:46 s:3.3.5] LMW may not overwrite its address register.
            PpuFuzzKind::Ordinary(K::Lmw) => ra != 0 && ra < rt,
            // [PPC-Book1 p:48 s:3.3.6] LSWI may not overwrite its address register.
            PpuFuzzKind::Ordinary(K::Lswi) => {
                let nb = if rb == 0 { 32 } else { rb };
                ra != 0 && !(0..nb.div_ceil(4)).any(|index| (rt + index) & 31 == ra)
            }
            // [PPC-Book1 p:48 s:3.3.6] LSWX cannot overwrite either address register first.
            PpuFuzzKind::Ordinary(K::Lswx) => rt != ra && rt != rb,
            // [PPC-Book1 p:104 s:4.6.2] Floating-point load-update requires nonzero RA.
            PpuFuzzKind::Ordinary(K::Lfsu | K::Lfdu | K::Lfsux | K::Lfdux) => ra != 0,
            // [PPC-Book1 p:40 s:3.3.3] Fixed-point store-update requires nonzero RA.
            PpuFuzzKind::Ordinary(
                K::Stwu | K::Stdu | K::Stbu | K::Sthu | K::Stdux | K::Sthux | K::Stwux | K::Stbux,
            ) => ra != 0,
            // [PPC-Book1 p:107 s:4.6] Floating-point store-update requires nonzero RA.
            PpuFuzzKind::Ordinary(K::Stfsu | K::Stfdu | K::Stfsux | K::Stfdux) => ra != 0,
            // [PPC-Book1 p:25 s:2.4] BCCTR cannot request CTR decrement.
            PpuFuzzKind::Ordinary(K::Bcctr) => rt & 0x04 != 0,
            _ => true,
        }
    }

    /// Reads the canonical values in operand order.
    pub fn canonical_parameters(&self) -> Vec<u32> {
        operand::canonical_parameters(self.canonical_word, &self.operands)
    }

    /// Produces one valid witness for each immediate boundary.
    pub fn immediate_boundary_words(&self) -> Vec<u32> {
        operand::immediate_boundary_words(self.canonical_word, &self.operands, |parameters| {
            self.encode(parameters).ok()
        })
    }

    /// Produces a valid word whose register operands share one encoded value.
    pub fn alias_word(&self, value: u32) -> Option<u32> {
        operand::register_alias_parameters(self.canonical_word, &self.operands, value)
            .and_then(|parameters| self.encode(&parameters).ok())
    }

    /// Produces exact-kind words by toggling non-operand bits.
    pub fn reserved_bit_words(&self) -> Vec<u32> {
        let original = crate::decode::decode(self.canonical_word).ok();
        let semantic_reserved = semantic_reserved_bits(self.kind);
        let operand_mask = operand::operand_mask(&self.operands);
        let discriminator_mask = discriminator_bits(self.kind, self.form) & !operand_mask;
        (0..u32::BITS)
            .filter_map(|bit| {
                let bit_mask = 1u32 << bit;
                if operand_mask & bit_mask != 0 || discriminator_mask & bit_mask != 0 {
                    return None;
                }
                let candidate = self.canonical_word ^ bit_mask;
                let ignored_by_decoder = crate::decode::decode(candidate).ok() == original;
                let reserved_operand =
                    semantic_reserved & bit_mask != 0 && exact_kind(candidate) == Some(self.kind);
                (ignored_by_decoder || reserved_operand).then_some(candidate)
            })
            .collect()
    }

    /// Produces valid exact-kind words by clearing one encoded operand bit.
    // [Regehr2012 p:4 s:5.2] A reducer that emits only variants it knows are valid never chases a difference an invalid variant caused.
    pub fn shrink(&self, raw: u32) -> Vec<u32> {
        operand::operand_bit_clears(raw, &self.operands)
            .into_iter()
            .filter(|&candidate| {
                crate::decode::decode(candidate).is_ok_and(|decoded| {
                    decoded.fuzz_descriptor(candidate).kind == self.kind
                        && generation_operands_are_valid(decoded)
                })
            })
            .collect()
    }
}
