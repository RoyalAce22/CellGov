//! The virtual UART group's registration, its state and the changes an
//! answer carries to it.

use std::collections::BTreeMap;

use cellgov_mem::lanes;
use cellgov_mem::scheme::{HashedSurface, Stamp};

use super::irq::{self, VirtualUartIrq};
use super::param;
use super::set_param::{self, VirtualUartPortParams};
use crate::group::state::StateChange;
use crate::group::table::Group;

/// The virtual UART group's registration.
pub(crate) const GROUP: Group = Group {
    name: "uart",
    hypercalls: &[irq::HYPERCALLS, param::HYPERCALLS, set_param::HYPERCALLS],
    lanes: &[240..=247],
    hashed_surfaces: &[
        HashedSurface {
            name: "the irq outlet id and the interrupt bitmap address of the virtual UART",
            source: Some(VIRTUAL_UART_IRQ_SOURCE),
            stamp: Stamp::Pending,
        },
        HashedSurface {
            name: "the triggers and the interrupt mask of each port of the virtual UART",
            source: Some(VIRTUAL_UART_PORT_PARAMS_SOURCE),
            stamp: Stamp::Pending,
        },
    ],
};

/// The lane source of the virtual UART's interrupt configuration: one
/// object, with key 0, when the partition configured it.
pub(crate) const VIRTUAL_UART_IRQ_SOURCE: u8 = 240;

/// The lane source of the values that `set_virtual_uart_param` gave the
/// ports: one object for each port in the state, keyed by its index in
/// port order.
pub(crate) const VIRTUAL_UART_PORT_PARAMS_SOURCE: u8 = 241;

/// The virtual UART group's state:
/// - the interrupt configuration of the partition's virtual UART;
/// - the triggers and the interrupt mask of each port.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UartState {
    irq: Option<VirtualUartIrq>,
    ports: BTreeMap<u64, VirtualUartPortParams>,
}

impl UartState {
    /// The virtual UART's interrupt configuration, or `None` before
    /// `configure_virtual_uart_irq` succeeds.
    pub fn irq(&self) -> Option<&VirtualUartIrq> {
        self.irq.as_ref()
    }

    /// The values of port `port`.
    ///
    /// A port with no entry has the values of
    /// [`VirtualUartPortParams::initial`]. Values equal to those remove
    /// the entry, so the state is the same for a port that got them and a
    /// port that got no call.
    pub fn port_params(&self, port: u64) -> VirtualUartPortParams {
        self.ports
            .get(&port)
            .copied()
            .unwrap_or(VirtualUartPortParams::initial(port))
    }

    /// Applies `change` to the state.
    pub(crate) fn apply(&mut self, change: UartChange) {
        match change {
            UartChange::IrqConfigured(irq) => self.irq = Some(irq),
            UartChange::PortParamsSet(params) => {
                if params == VirtualUartPortParams::initial(params.port) {
                    self.ports.remove(&params.port);
                } else {
                    self.ports.insert(params.port, params);
                }
            }
        }
    }

    /// The sync-state terms of the interrupt configuration and of the
    /// ports' values. A state with neither adds no term.
    pub fn sync_term(&self) -> u128 {
        let irq = self
            .irq
            .map_or(0, |irq| lanes::value_term(VIRTUAL_UART_IRQ_SOURCE, 0, &irq));
        (0u64..)
            .zip(self.ports.values())
            .fold(irq, |sum, (index, params)| {
                sum.wrapping_add(lanes::value_term(
                    VIRTUAL_UART_PORT_PARAMS_SOURCE,
                    index,
                    params,
                ))
            })
    }
}

/// A change to the virtual UART group's state that an answer carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UartChange {
    /// The call gave the virtual UART this interrupt configuration.
    IrqConfigured(VirtualUartIrq),
    /// The call gave a port these values.
    PortParamsSet(VirtualUartPortParams),
}

impl From<UartChange> for StateChange {
    fn from(change: UartChange) -> Self {
        StateChange::Uart(Box::new(change))
    }
}
