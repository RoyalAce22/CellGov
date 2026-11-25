//! The interrupt group's registration, its state and the changes an
//! answer carries to it.

use cellgov_mem::scheme::{HashedSurface, Stamp};

use super::bitmap::{self, IrqStateBitmapRegistration, IrqStateBitmapState};
use super::event_port::{self, EventReceivePort, EventReceivePortState};
use super::irq_plug;
use crate::group::state::StateChange;
use crate::group::table::Group;

/// The interrupt group's registration.
pub(crate) const GROUP: Group = Group {
    name: "interrupt",
    hypercalls: &[
        bitmap::HYPERCALLS,
        event_port::HYPERCALLS,
        irq_plug::HYPERCALLS,
    ],
    lanes: &[81..=82, 144..=159],
    hashed_surfaces: &[
        HashedSurface {
            name: "the interrupt state bitmaps",
            source: Some(IRQ_STATE_BITMAP_SOURCE),
            stamp: Stamp::Version(24),
        },
        HashedSurface {
            name: "the event receive ports",
            source: Some(EVENT_RECEIVE_PORT_SOURCE),
            stamp: Stamp::Version(25),
        },
        HashedSurface {
            name: "the plug connection of each event receive port",
            source: Some(EVENT_RECEIVE_PORT_SOURCE),
            stamp: Stamp::Version(26),
        },
        HashedSurface {
            name: "the irq outlets of the logical SPEs and their plug connections",
            source: Some(IRQ_OUTLET_SOURCE),
            stamp: Stamp::Pending,
        },
    ],
};

/// The lane source of the interrupt state bitmaps registered with the
/// hypervisor, one object per logical PPE thread in key order.
pub(crate) const IRQ_STATE_BITMAP_SOURCE: u8 = 81;

/// The lane source of the event receive ports the hypervisor
/// constructed, one object per port in id order.
pub(crate) const EVENT_RECEIVE_PORT_SOURCE: u8 = 82;

/// The lane source of the irq outlets of the logical SPEs, one object per
/// outlet in id order.
pub(crate) const IRQ_OUTLET_SOURCE: u8 = 144;

/// The interrupt group's state: the interrupt state bitmaps, the event
/// receive ports and the irq outlets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InterruptGroupState {
    bitmaps: IrqStateBitmapState,
    ports: EventReceivePortState,
}

impl InterruptGroupState {
    /// The interrupt state bitmap that each logical PPE thread
    /// registered.
    pub fn bitmaps(&self) -> &IrqStateBitmapState {
        &self.bitmaps
    }

    /// The partition's event receive ports and the irq outlets of its
    /// logical SPEs.
    pub fn ports(&self) -> &EventReceivePortState {
        &self.ports
    }

    /// Applies `change`.
    pub(crate) fn apply(&mut self, change: InterruptChange) {
        match change {
            InterruptChange::BitmapRegistered(registration) => self.bitmaps.register(registration),
            InterruptChange::PortConstructed(port) => self.ports.construct(port),
            InterruptChange::PlugConnect(port) => self.ports.connect(port),
            InterruptChange::OutletConstructed(outlet) => self.ports.construct_outlet(outlet),
            InterruptChange::OutletPlugConnect(outlet) => self.ports.connect_outlet(outlet),
        }
    }

    /// The sync-state terms of the bitmaps, the ports and the irq outlets.
    pub fn sync_term(&self) -> u128 {
        self.bitmaps
            .sync_term()
            .wrapping_add(self.ports.sync_term())
    }
}

/// A change to the partition's interrupt state that an answer carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptChange {
    /// The call registered this interrupt state bitmap.
    BitmapRegistered(IrqStateBitmapRegistration),
    /// The call constructed this event receive port.
    PortConstructed(EventReceivePort),
    /// The call's connect step left the port with this id in this state:
    /// a new plug, and a connection when the plug was free.
    PlugConnect(EventReceivePort),
    /// The call constructed this irq outlet of a logical SPE.
    OutletConstructed(EventReceivePort),
    /// The call's connect step left the irq outlet with this id in this
    /// state, as [`Self::PlugConnect`] leaves a port.
    OutletPlugConnect(EventReceivePort),
}

impl From<InterruptChange> for StateChange {
    fn from(change: InterruptChange) -> Self {
        StateChange::Interrupt(Box::new(change))
    }
}
