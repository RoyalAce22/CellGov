//! Shared field coverage for independent execution references: the
//! field status, the source of a reference, and the one comparison rule
//! the PPU and SPU references apply per field.

use serde::{Deserialize, Serialize};

/// A represented value or an explicit reason that comparison is invalid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReferenceField<T> {
    /// The source represents this field.
    Value {
        /// Expected value.
        value: T,
    },
    /// The architecture leaves this field undefined.
    // [Jiang2022 p:7 s:4.2] Most device and emulator inconsistencies trace to behaviour the manual leaves undefined, so the comparison skips such a field.
    Undefined {
        /// Source-specific reason.
        reason: String,
    },
    /// The source cannot represent this field.
    Unsupported {
        /// Source-specific reason.
        reason: String,
    },
}

impl<T> ReferenceField<T> {
    /// The represented value, or `None` for an omitted field.
    pub(crate) fn as_value(&self) -> Option<&T> {
        match self {
            Self::Value { value } => Some(value),
            Self::Undefined { .. } | Self::Unsupported { .. } => None,
        }
    }

    /// The omission of a field whose reason is blank.
    pub(crate) fn blank_reason(&self) -> Option<ReferenceOmission> {
        match self {
            Self::Value { .. } => None,
            Self::Undefined { reason } => reason
                .trim()
                .is_empty()
                .then_some(ReferenceOmission::Undefined),
            Self::Unsupported { reason } => reason
                .trim()
                .is_empty()
                .then_some(ReferenceOmission::Unsupported),
        }
    }
}

/// Why a reference field takes no part in a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceOmission {
    /// The architecture leaves the field undefined.
    Undefined,
    /// The source cannot represent the field.
    Unsupported,
}

impl ReferenceOmission {
    /// The status as an artifact writes it.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Undefined => "undefined",
            Self::Unsupported => "unsupported",
        }
    }
}

/// Independent source of a reference observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReferenceProvenance {
    /// A vector derived from a cited public architecture rule.
    DocumentedVector {
        /// Official source key, printed page, and section.
        citation: String,
        /// Stable identifier for the chosen source inputs.
        vector_id: String,
    },
    /// An operator-supplied observation from physical hardware.
    HardwareCapture {
        /// Stable operator capture identifier.
        capture_id: String,
        /// Hardware model supplied by the operator.
        device: String,
        /// Firmware and acquisition context supplied by the operator.
        environment: String,
        /// SHA-256 of the original capture artifact.
        source_sha256: String,
    },
}

/// The first provenance field that fails its check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProvenanceFault<'a> {
    /// A required text field is blank.
    Blank(&'static str),
    /// The citation fails the architecture's citation rule.
    Citation(&'a str),
    /// The capture digest is not 64 lowercase hexadecimal digits.
    Digest,
}

impl ReferenceProvenance {
    /// Checks the fields in declaration order; `citation_ok` is the
    /// architecture's citation rule.
    pub(crate) fn check(
        &self,
        citation_ok: impl FnOnce(&str) -> bool,
    ) -> Result<(), ProvenanceFault<'_>> {
        match self {
            Self::DocumentedVector {
                citation,
                vector_id,
            } => {
                require_text("vector_id", vector_id)?;
                if !citation_ok(citation) {
                    return Err(ProvenanceFault::Citation(citation));
                }
            }
            Self::HardwareCapture {
                capture_id,
                device,
                environment,
                source_sha256,
            } => {
                require_text("capture_id", capture_id)?;
                require_text("device", device)?;
                require_text("environment", environment)?;
                if !is_lower_hex(source_sha256, 64) {
                    return Err(ProvenanceFault::Digest);
                }
            }
        }
        Ok(())
    }
}

fn require_text(field: &'static str, value: &str) -> Result<(), ProvenanceFault<'static>> {
    if value.trim().is_empty() {
        return Err(ProvenanceFault::Blank(field));
    }
    Ok(())
}

/// Whether `value` is exactly `digits` lowercase hexadecimal digits.
pub(crate) fn is_lower_hex(value: &str, digits: usize) -> bool {
    value.len() == digits
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// A comparison that records each field's verdict under a component.
pub(crate) trait ReferenceComparison {
    /// The component a field is recorded under.
    type Component: Copy;
    /// What a disagreement records.
    type Difference;
    /// Records a field with a represented value.
    fn compared(&mut self, component: Self::Component);
    /// Records a represented field that disagreed.
    fn differs(&mut self, component: Self::Component, difference: Self::Difference);
    /// Records a field the source omits.
    fn omitted(&mut self, component: Self::Component, omission: ReferenceOmission, reason: &str);
}

/// Records one field: compared, and checked with `difference`, when the
/// source represents it; omitted with its reason otherwise.
// [Jiang2022 p:7 s:4.2] Most device and emulator inconsistencies trace to behaviour the manual leaves undefined, so a component the documentation marks undefined is excluded rather than counted as a difference.
// [McKeeman1998 p:101 s:Differential Testing] Two results can differ and both be correct where the standard leaves a construct undefined, so a field the documentation marks undefined is excluded from the comparison.
pub(crate) fn compare_field<C: ReferenceComparison, T>(
    comparison: &mut C,
    component: C::Component,
    field: &ReferenceField<T>,
    difference: impl FnOnce(&T) -> Option<C::Difference>,
) {
    match field {
        ReferenceField::Value { value } => {
            comparison.compared(component);
            if let Some(difference) = difference(value) {
                comparison.differs(component, difference);
            }
        }
        ReferenceField::Undefined { reason } => {
            comparison.omitted(component, ReferenceOmission::Undefined, reason);
        }
        ReferenceField::Unsupported { reason } => {
            comparison.omitted(component, ReferenceOmission::Unsupported, reason);
        }
    }
}

#[cfg(test)]
#[path = "tests/reference_tests.rs"]
mod tests;
