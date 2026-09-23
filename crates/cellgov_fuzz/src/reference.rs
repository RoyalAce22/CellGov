//! Shared field coverage for independent execution references.

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
