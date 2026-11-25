//! lv1's registration of a storage device for the game OS partition, and
//! the repository nodes that it writes.
//!
//! The facts are in [`cellgov_ps3_abi::lv1::storage_registration`]. lv1
//! registers a device before it reports the device to the game OS through
//! the notification device. [`register_device`] gives the change of a
//! registration, which the answer to a notification read carries. After
//! that change, [`registered_node`] answers the repository reads of the
//! nodes that the registration wrote.

use cellgov_ps3_abi::lv1::repository::RepositoryKey;
use cellgov_ps3_abi::lv1::storage_registration::registration;

use super::registration::{StorageChange, StorageState};
use crate::answer::UnmodelledError;

/// The registration at a firmware whose registration no source gives.
const UNSOURCED_FIRMWARE: UnmodelledError = UnmodelledError::new(
    "a storage device registration at a firmware whose registration values no source gives",
);

/// The registration of a device that the captured bus table does not
/// hold.
const UNKNOWN_DEVICE: UnmodelledError = UnmodelledError::new(
    "a storage device registration of a device that the captured bus table does not hold",
);

/// The change that lv1's registration of the storage device `device`
/// makes at the firmware version `firmware`.
///
/// The registration is a refusal in these cases:
/// - no source gives the registration's values for `firmware`;
/// - `firmware` is `None`;
/// - the bus table of the source holds no device of id `device`.
pub(crate) fn register_device(
    firmware: Option<&str>,
    device: u64,
) -> Result<StorageChange, UnmodelledError> {
    let registration = firmware.and_then(registration).ok_or(UNSOURCED_FIRMWARE)?;
    let device = registration.device(device).ok_or(UNKNOWN_DEVICE)?;
    Ok(StorageChange::DeviceRegistered(device.id))
}

/// The two values of the node `key` that lv1's registrations wrote, or
/// `None` when no registration of `storage` wrote the key.
///
/// The answer is also `None` at a firmware whose registration no source
/// gives.
pub(crate) fn registered_node(
    storage: &StorageState,
    firmware: Option<&str>,
    key: &RepositoryKey,
) -> Option<(u64, u64)> {
    let registration = firmware.and_then(registration)?;
    registration.node(storage.registered_devices(), key)
}

#[cfg(test)]
#[path = "tests/device_registration_tests.rs"]
mod tests;
