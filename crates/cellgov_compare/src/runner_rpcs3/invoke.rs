//! Launch RPCS3 headless and wait for exit or timeout.

use std::process::Command;
use std::time::Duration;

use crate::observation::ObservedOutcome;

use super::config::{Rpcs3Config, Rpcs3TestConfig};
use super::error::Rpcs3Error;

/// Launch RPCS3 and wait for exit or timeout. Returns the mapped outcome.
pub(super) fn invoke(
    config: &Rpcs3Config,
    test: &Rpcs3TestConfig,
) -> Result<ObservedOutcome, Rpcs3Error> {
    let mut child = Command::new(&config.executable)
        .arg("--headless")
        .arg(&test.binary)
        .spawn()
        .map_err(Rpcs3Error::Launch)?;

    let deadline = std::time::Instant::now() + test.timeout;
    let exit_status = loop {
        match child.try_wait().map_err(Rpcs3Error::Launch)? {
            Some(status) => break status,
            None => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(ObservedOutcome::Timeout);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    };

    let outcome = if exit_status.success() {
        ObservedOutcome::Completed
    } else {
        ObservedOutcome::Fault
    };
    Ok(outcome)
}
