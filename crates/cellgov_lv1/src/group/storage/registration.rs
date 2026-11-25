//! The storage group's registration, its state and the changes an
//! answer carries to it.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_mem::lanes::{self, LaneValue, ObjectLanes};
use cellgov_mem::scheme::{HashedSurface, Stamp};

use super::{read, status, write};
use crate::group::state::StateChange;
use crate::group::table::Group;

/// The storage group's registration.
pub(crate) const GROUP: Group = Group {
    name: "storage",
    hypercalls: &[read::HYPERCALLS, write::HYPERCALLS, status::HYPERCALLS],
    lanes: &[208..=215],
    hashed_surfaces: &[
        HashedSurface {
            name: "the notification requests that lv1 holds for the partition",
            source: Some(NOTIFY_REQUEST_SOURCE),
            stamp: Stamp::Version(33),
        },
        HashedSurface {
            name: "the event mask of the partition's storage bus open",
            source: Some(NOTIFY_EVENT_MASK_SOURCE),
            stamp: Stamp::Version(33),
        },
        HashedSurface {
            name: "the storage devices that lv1 registered for the partition",
            source: Some(REGISTERED_DEVICE_SOURCE),
            stamp: Stamp::Version(34),
        },
        HashedSurface {
            name: "the number of storage devices that lv1 reported to the partition",
            source: Some(REPORTED_DEVICES_SOURCE),
            stamp: Stamp::Version(34),
        },
        HashedSurface {
            name: "the notification read that lv1 holds waiting for the partition",
            source: Some(WAITING_READ_SOURCE),
            stamp: Stamp::Version(34),
        },
    ],
};

/// The lane source of the notification requests: one object for each
/// request, keyed by its tag.
pub(crate) const NOTIFY_REQUEST_SOURCE: u8 = 208;

/// The lane source of the event mask: object 0 while lv1 holds a mask
/// for the partition.
pub(crate) const NOTIFY_EVENT_MASK_SOURCE: u8 = 209;

/// The lane source of the registered storage devices: one object for
/// each device, keyed by its id.
pub(crate) const REGISTERED_DEVICE_SOURCE: u8 = 210;

/// The lane source of the number of reported devices: object 0 while the
/// number is not 0.
pub(crate) const REPORTED_DEVICES_SOURCE: u8 = 211;

/// The lane source of the waiting notification read: object 0 while lv1
/// holds one.
pub(crate) const WAITING_READ_SOURCE: u8 = 212;

/// A notification read that lv1 holds waiting for a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaitingRead {
    /// The real address of CellGov's storage of the read's buffer. lv1
    /// stores its own real address of the buffer at +72 of the
    /// partition's open record. The two are equal for a real mode buffer.
    /// CellGov keeps an allocated region's pages in one range of storage.
    pub buffer: u64,
    /// The tag of the read's request, at +80 of the open record.
    pub tag: u8,
}

impl LaneValue for WaitingRead {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.buffer);
        lanes.lane(2, 0, u64::from(self.tag));
    }
}

/// A notification request that lv1 holds for the partition on the
/// storage bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotifyRequest {
    /// The tag that lv1 gave the request, below 16.
    pub tag: u8,
    /// The status with which lv1 completed the request, or `None` while
    /// the request waits.
    pub completion: Option<u64>,
}

impl LaneValue for NotifyRequest {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.tag));
        if let Some(status) = self.completion {
            lanes.lane(2, 0, 1);
            lanes.lane(3, 0, status);
        }
    }
}

/// The storage group's state.
///
/// The state holds these values:
/// - each notification request that lv1 holds for the partition;
/// - the event mask that lv1 stored in the partition's storage bus open;
/// - the id of each storage device that lv1 registered for the
///   partition;
/// - the number of storage devices that lv1 reported to the partition,
///   the index at +64 of its open record;
/// - the notification read that lv1 holds waiting for the partition.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StorageState {
    requests: BTreeMap<u8, NotifyRequest>,
    event_mask: Option<u64>,
    registered: BTreeSet<u64>,
    reported: u64,
    waiting: Option<WaitingRead>,
}

impl StorageState {
    /// Each notification request, in tag order.
    pub fn notify_requests(&self) -> impl Iterator<Item = NotifyRequest> + '_ {
        self.requests.values().copied()
    }

    /// The event mask of the partition's storage bus open, or `None` when
    /// lv1 stored no mask.
    pub fn notify_event_mask(&self) -> Option<u64> {
        self.event_mask
    }

    /// The id of each storage device that lv1 registered for the
    /// partition, ascending.
    pub fn registered_devices(&self) -> impl Iterator<Item = u64> + '_ {
        self.registered.iter().copied()
    }

    /// The number of storage devices that lv1 reported to the partition
    /// through notification reads.
    pub fn reported_devices(&self) -> u64 {
        self.reported
    }

    /// The notification read that lv1 holds waiting, or `None` when no
    /// read waits.
    pub fn waiting_read(&self) -> Option<WaitingRead> {
        self.waiting
    }

    /// The tags of the partition's notification requests, one bit for
    /// each tag.
    pub fn held_tags(&self) -> u16 {
        self.requests.keys().fold(0, |tags, &tag| {
            tags | 1u16.checked_shl(u32::from(tag)).unwrap_or(0)
        })
    }

    /// Applies `change` to the state.
    pub(crate) fn apply(&mut self, change: StorageChange) {
        match change {
            StorageChange::NotifyEventMaskSet(mask) => {
                self.event_mask = Some(mask);
            }
            StorageChange::NotifyRequested(request) => {
                self.requests.insert(request.tag, request);
            }
            StorageChange::NotifyRequestTaken(tag) => {
                self.requests.remove(&tag);
            }
            StorageChange::DeviceRegistered(device) => {
                self.registered.insert(device);
            }
            StorageChange::NotifyDeviceReported => {
                self.reported = self.reported.wrapping_add(1);
                self.waiting = None;
            }
            StorageChange::NotifyReadWaiting(read) => {
                self.waiting = Some(read);
            }
        }
    }

    /// The sum of the sync-state terms of the group's hashed surfaces.
    ///
    /// The default state gives 0.
    pub fn sync_term(&self) -> u128 {
        let requests = self.notify_requests().fold(0u128, |sum, request| {
            sum.wrapping_add(lanes::value_term(
                NOTIFY_REQUEST_SOURCE,
                u64::from(request.tag),
                &request,
            ))
        });
        let masked = self.event_mask.map_or(requests, |mask| {
            requests.wrapping_add(lanes::value_term(NOTIFY_EVENT_MASK_SOURCE, 0, &mask))
        });
        let registered = self.registered_devices().fold(masked, |sum, device| {
            sum.wrapping_add(lanes::value_term(REGISTERED_DEVICE_SOURCE, device, &device))
        });
        let reported = match self.reported {
            0 => registered,
            count => registered.wrapping_add(lanes::value_term(REPORTED_DEVICES_SOURCE, 0, &count)),
        };
        self.waiting.map_or(reported, |read| {
            reported.wrapping_add(lanes::value_term(WAITING_READ_SOURCE, 0, &read))
        })
    }
}

/// A change to the storage group's state that an answer carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageChange {
    /// lv1 stored this event mask in the partition's storage bus open.
    NotifyEventMaskSet(u64),
    /// lv1 holds this notification request for the partition.
    NotifyRequested(NotifyRequest),
    /// lv1 gave the partition the completed request of this tag, and
    /// freed the tag.
    NotifyRequestTaken(u8),
    /// lv1 registered the storage device of this id for the partition,
    /// and wrote the device's repository nodes.
    DeviceRegistered(u64),
    /// lv1 reported the next storage device to the partition through a
    /// notification read. It raised the index at +64 of the open record,
    /// and stored 0 at +72, so no read waits.
    NotifyDeviceReported,
    /// lv1 holds this notification read waiting for a report.
    NotifyReadWaiting(WaitingRead),
}

impl From<StorageChange> for StateChange {
    fn from(change: StorageChange) -> Self {
        StateChange::Storage(Box::new(change))
    }
}

#[cfg(test)]
#[path = "tests/notify_hash_tests.rs"]
mod hash_tests;

#[cfg(test)]
#[path = "tests/registered_device_hash_tests.rs"]
mod registered_device_hash_tests;

#[cfg(test)]
#[path = "tests/notify_read_hash_tests.rs"]
mod notify_read_hash_tests;
