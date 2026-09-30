//! The stop-state query and the restart of a unit that stopped itself.

use cellgov_event::UnitId;
use cellgov_exec::{RestartError, StopRegisters};

use super::Runtime;

impl Runtime {
    /// The stopped state `unit` holds, or `None` for a unit that did not
    /// stop itself or is not registered.
    pub fn unit_stop_registers(&self, unit: UnitId) -> Option<StopRegisters> {
        self.registry.get(unit)?.stop_registers()
    }

    /// Resume a unit that stopped itself, at the address its stopped
    /// state names.
    ///
    /// # Errors
    ///
    /// - [`RestartError::UnknownUnit`] when no unit has the id.
    /// - [`RestartError::Retired`] when a `Finished` status override holds
    ///   the unit, as a process exit sets for every unit of the process.
    /// - [`RestartError::NotStopped`] when the unit holds no stopped
    ///   state.
    pub fn restart_unit(&mut self, unit: UnitId) -> Result<(), RestartError> {
        if self.registry.status_override(unit) == Some(cellgov_exec::UnitStatus::Finished) {
            return Err(RestartError::Retired);
        }
        self.registry
            .get_mut(unit)
            .ok_or(RestartError::UnknownUnit)?
            .restart()
    }
}

#[cfg(test)]
#[path = "tests/unit_stop_tests.rs"]
mod tests;
