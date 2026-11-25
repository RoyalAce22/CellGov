//! The virtual UART group's registration, its state and the changes an
//! answer carries to it.

use cellgov_mem::lanes;
use cellgov_mem::scheme::{HashedSurface, Stamp};

use super::irq::{self, VirtualUartIrq};
use crate::group::state::StateChange;
use crate::group::table::Group;

/// The virtual UART group's registration.
pub(crate) const GROUP: Group = Group {
    name: "uart",
    hypercalls: &[irq::HYPERCALLS],
    lanes: &[240..=247],
    hashed_surfaces: &[HashedSurface {
        name: "the irq outlet id and the interrupt bitmap address of the virtual UART",
        source: Some(VIRTUAL_UART_IRQ_SOURCE),
        stamp: Stamp::Pending,
    }],
};

/// The lane source of the virtual UART's interrupt configuration: one
/// object, with key 0, when the partition configured it.
pub(crate) const VIRTUAL_UART_IRQ_SOURCE: u8 = 240;

/// The virtual UART group's state: the interrupt configuration of the
/// partition's virtual UART.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UartState {
    irq: Option<VirtualUartIrq>,
}

impl UartState {
    /// The virtual UART's interrupt configuration, or `None` before
    /// `configure_virtual_uart_irq` succeeds.
    pub fn irq(&self) -> Option<&VirtualUartIrq> {
        self.irq.as_ref()
    }

    /// Applies `change` to the state.
    pub(crate) fn apply(&mut self, change: UartChange) {
        match change {
            UartChange::IrqConfigured(irq) => self.irq = Some(irq),
        }
    }

    /// The sync-state term of the interrupt configuration. A state with
    /// no configuration adds no term.
    pub fn sync_term(&self) -> u128 {
        self.irq
            .map_or(0, |irq| lanes::value_term(VIRTUAL_UART_IRQ_SOURCE, 0, &irq))
    }
}

/// A change to the virtual UART group's state that an answer carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UartChange {
    /// The call gave the virtual UART this interrupt configuration.
    IrqConfigured(VirtualUartIrq),
}

impl From<UartChange> for StateChange {
    fn from(change: UartChange) -> Self {
        StateChange::Uart(Box::new(change))
    }
}
