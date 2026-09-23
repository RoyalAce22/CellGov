//! This module defines deterministic parameter streams for structural instruction generation.

use serde::{Deserialize, Serialize};

use crate::GeneratorError;

/// Supplies ordered choices to one interpreter-owned encoding descriptor.
///
/// Untyped parameter mutations become structural input mutations after a descriptor maps each
/// value to its declared operand field. [Padhye2019 p:329 s:Abstract]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ParameterStream(Vec<u32>);

impl ParameterStream {
    /// Creates a stream from stable integer choices.
    pub fn new(values: Vec<u32>) -> Self {
        Self(values)
    }

    /// Returns the choices in descriptor order.
    pub fn values(&self) -> &[u32] {
        &self.0
    }

    /// Replaces one structural choice.
    ///
    /// # Errors
    ///
    /// Returns [`GeneratorError::ParameterIndex`] if `index` is outside the stream.
    pub fn mutate(&mut self, index: usize, value: u32) -> Result<(), GeneratorError> {
        let length = self.0.len();
        let Some(parameter) = self.0.get_mut(index) else {
            return Err(GeneratorError::ParameterIndex { index, length });
        };
        *parameter = value;
        Ok(())
    }
}
