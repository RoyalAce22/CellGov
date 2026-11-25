//! The interrupt group's registration, its state and the changes an
//! answer carries to it.

use cellgov_mem::scheme::{HashedSurface, Stamp};

use super::bitmap::{self, IrqStateBitmapRegistration, IrqStateBitmapState, StatusShadowWrite};
use super::event_port::{self, EventReceivePort, EventReceivePortState};
use super::pause::{self, PausedThread, PausedThreadState};
use super::{irq_plug, mask, send_event};
use crate::group::state::StateChange;
use crate::group::table::Group;

/// The interrupt group's registration.
pub(crate) const GROUP: Group = Group {
    name: "interrupt",
    hypercalls: &[
        bitmap::HYPERCALLS,
        event_port::HYPERCALLS,
        irq_plug::HYPERCALLS,
        mask::HYPERCALLS,
        pause::HYPERCALLS,
        send_event::HYPERCALLS,
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
            stamp: Stamp::Version(28),
        },
        HashedSurface {
            name: "the irq outlet of the virtual UART and its plug connection",
            source: Some(IRQ_OUTLET_SOURCE),
            stamp: Stamp::Version(29),
        },
        HashedSurface {
            name: "the paused PPE hardware threads and their decrementer wake flags",
            source: Some(PAUSED_THREAD_SOURCE),
            stamp: Stamp::Version(28),
        },
        HashedSurface {
            name: "the event receive ports with a pending event",
            source: Some(PENDING_EVENT_SOURCE),
            stamp: Stamp::Version(29),
        },
        HashedSurface {
            name: "the status shadows of the logical PPE threads",
            source: Some(IRQ_STATUS_SHADOW_SOURCE),
            stamp: Stamp::Version(29),
        },
        HashedSurface {
            name: "the paused PPE hardware threads that lv1 woke",
            source: Some(WOKEN_THREAD_SOURCE),
            stamp: Stamp::Version(29),
        },
    ],
};

/// The lane source of the interrupt state bitmaps registered with the
/// hypervisor, one object per logical PPE thread in key order.
pub(crate) const IRQ_STATE_BITMAP_SOURCE: u8 = 81;

/// The lane source of the event receive ports the hypervisor
/// constructed, one object per port in id order.
pub(crate) const EVENT_RECEIVE_PORT_SOURCE: u8 = 82;

/// The lane source of the irq outlets of the logical SPEs and the virtual
/// UART, one object per outlet in id order.
pub(crate) const IRQ_OUTLET_SOURCE: u8 = 144;

/// The lane source of the PPE hardware threads that lv1 paused, one
/// object per thread in thread order.
pub(crate) const PAUSED_THREAD_SOURCE: u8 = 145;

/// The lane source of the event receive ports with a pending event, one
/// object per port in id order.
pub(crate) const PENDING_EVENT_SOURCE: u8 = 146;

/// The lane source of the status shadows with a set bit, one object per
/// logical PPE thread in key order.
pub(crate) const IRQ_STATUS_SHADOW_SOURCE: u8 = 147;

/// The lane source of the paused threads that lv1 woke, one object per
/// thread in thread order.
pub(crate) const WOKEN_THREAD_SOURCE: u8 = 148;

/// The interrupt group's state.
///
/// The state holds:
/// - the interrupt state bitmaps and the status shadows;
/// - the event receive ports, the irq outlets and the pending events;
/// - the paused threads and their wakes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InterruptGroupState {
    bitmaps: IrqStateBitmapState,
    ports: EventReceivePortState,
    paused: PausedThreadState,
}

impl InterruptGroupState {
    /// The interrupt state bitmap that each logical PPE thread
    /// registered, and the status shadow of each thread.
    pub fn bitmaps(&self) -> &IrqStateBitmapState {
        &self.bitmaps
    }

    /// The partition's event receive ports and its irq outlets.
    pub fn ports(&self) -> &EventReceivePortState {
        &self.ports
    }

    /// The PPE hardware threads that lv1 paused, and the paused threads
    /// that lv1 woke.
    pub fn paused(&self) -> &PausedThreadState {
        &self.paused
    }

    /// Applies `change`.
    pub(crate) fn apply(&mut self, change: InterruptChange) {
        match change {
            InterruptChange::BitmapRegistered(registration) => self.bitmaps.register(registration),
            InterruptChange::PortConstructed(port) => self.ports.construct(port),
            InterruptChange::PlugConnect(port) => self.ports.connect(port),
            InterruptChange::OutletConstructed(outlet) => self.ports.construct_outlet(outlet),
            InterruptChange::OutletPlugConnect(outlet) => self.ports.connect_outlet(outlet),
            InterruptChange::ThreadPaused(paused) => self.paused.pause(paused),
            InterruptChange::EventPending(id) => self.ports.set_pending(id),
            InterruptChange::StatusShadow(write) => self.bitmaps.write_shadow(write),
            InterruptChange::ThreadWoken(thread) => self.paused.wake(thread),
        }
    }

    /// The sync-state terms of the bitmaps, the ports, the irq outlets and
    /// the paused threads.
    pub fn sync_term(&self) -> u128 {
        self.bitmaps
            .sync_term()
            .wrapping_add(self.ports.sync_term())
            .wrapping_add(self.paused.sync_term())
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
    /// The call constructed this irq outlet of a logical SPE or of the
    /// virtual UART.
    OutletConstructed(EventReceivePort),
    /// The call's connect step left the irq outlet with this id in this
    /// state, as [`Self::PlugConnect`] leaves a port.
    OutletPlugConnect(EventReceivePort),
    /// The call paused this thread.
    ThreadPaused(PausedThread),
    /// The call set the pending byte of the port with this id.
    EventPending(u64),
    /// The call left this doubleword in a thread's status shadow.
    StatusShadow(StatusShadowWrite),
    /// lv1 cleared the pause flag of this paused hardware thread.
    ThreadWoken(u8),
}

impl From<InterruptChange> for StateChange {
    fn from(change: InterruptChange) -> Self {
        StateChange::Interrupt(Box::new(change))
    }
}
