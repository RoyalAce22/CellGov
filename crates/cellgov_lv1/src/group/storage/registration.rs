//! The storage group's registration, its state and the changes an
//! answer carries to it.

use std::collections::BTreeMap;

use cellgov_mem::lanes::{self, LaneValue, ObjectLanes};
use cellgov_mem::scheme::{HashedSurface, Stamp};

use super::write;
use crate::group::state::StateChange;
use crate::group::table::Group;

/// The storage group's registration.
pub(crate) const GROUP: Group = Group {
    name: "storage",
    hypercalls: &[write::HYPERCALLS],
    lanes: &[208..=215],
    hashed_surfaces: &[
        HashedSurface {
            name: "the notification requests that lv1 holds for the partition",
            source: Some(NOTIFY_REQUEST_SOURCE),
            stamp: Stamp::Pending,
        },
        HashedSurface {
            name: "the event mask of the partition's storage bus open",
            source: Some(NOTIFY_EVENT_MASK_SOURCE),
            stamp: Stamp::Pending,
        },
    ],
};

/// The lane source of the notification requests: one object for each
/// request, keyed by its tag.
pub(crate) const NOTIFY_REQUEST_SOURCE: u8 = 208;

/// The lane source of the event mask: object 0 while lv1 holds a mask
/// for the partition.
pub(crate) const NOTIFY_EVENT_MASK_SOURCE: u8 = 209;

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
/// - the event mask that lv1 stored in the partition's storage bus open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StorageState {
    requests: BTreeMap<u8, NotifyRequest>,
    event_mask: Option<u64>,
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
        }
    }

    /// The sum of the sync-state terms of the group's hashed surfaces.
    ///
    /// A state that holds no value adds no term.
    pub fn sync_term(&self) -> u128 {
        let requests = self.notify_requests().fold(0u128, |sum, request| {
            sum.wrapping_add(lanes::value_term(
                NOTIFY_REQUEST_SOURCE,
                u64::from(request.tag),
                &request,
            ))
        });
        self.event_mask.map_or(requests, |mask| {
            requests.wrapping_add(lanes::value_term(NOTIFY_EVENT_MASK_SOURCE, 0, &mask))
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
}

impl From<StorageChange> for StateChange {
    fn from(change: StorageChange) -> Self {
        StateChange::Storage(Box::new(change))
    }
}

#[cfg(test)]
#[path = "tests/notify_hash_tests.rs"]
mod hash_tests;
