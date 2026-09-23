//! Panic classification at narrow target-call boundaries.

use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Stable panic payload retained without addresses or `Debug` formatting.
///
/// Process-dependent text such as an address would make equal failures
/// look distinct, so the payload omits it from the identity.
/// [Chen2013 p:3 s:2.3 Distance Functions for Test Cases]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum TargetPanicPayload {
    /// A static string payload.
    StaticStr(String),
    /// An owned string payload.
    String(String),
    /// A non-string payload.
    NonString,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("fuzz harness panicked outside the target boundary")]
pub(crate) struct HarnessPanic;

pub(crate) fn call_target<T>(call: impl FnOnce() -> T) -> Result<T, TargetPanicPayload> {
    catch_unwind(AssertUnwindSafe(call)).map_err(classify_payload)
}

pub(crate) fn call_harness<T>(call: impl FnOnce() -> T) -> Result<T, HarnessPanic> {
    catch_unwind(AssertUnwindSafe(call)).map_err(|_| HarnessPanic)
}

fn classify_payload(payload: Box<dyn Any + Send>) -> TargetPanicPayload {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        TargetPanicPayload::StaticStr((*message).to_owned())
    } else if let Some(message) = payload.downcast_ref::<String>() {
        TargetPanicPayload::String(message.clone())
    } else {
        TargetPanicPayload::NonString
    }
}

#[cfg(test)]
#[path = "tests/boundary_tests.rs"]
mod tests;
