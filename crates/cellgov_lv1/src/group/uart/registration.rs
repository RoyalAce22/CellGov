//! The virtual UART group's registration, its state and the changes an
//! answer carries to it.

use std::collections::BTreeMap;

use cellgov_mem::lanes;
use cellgov_mem::scheme::{HashedSurface, Stamp};

use super::irq::{self, VirtualUartIrq};
use super::param;
use super::read;
use super::receive::ReceivedBytes;
use super::request::PendingRequest;
use super::set_param::{self, VirtualUartPortParams};
use super::write;
use crate::group::state::StateChange;
use crate::group::table::Group;

/// The virtual UART group's registration.
pub(crate) const GROUP: Group = Group {
    name: "uart",
    hypercalls: &[
        irq::HYPERCALLS,
        param::HYPERCALLS,
        set_param::HYPERCALLS,
        read::HYPERCALLS,
        write::HYPERCALLS,
    ],
    lanes: &[240..=247],
    hashed_surfaces: &[
        HashedSurface {
            name: "the irq outlet id and the interrupt bitmap address of the virtual UART",
            source: Some(VIRTUAL_UART_IRQ_SOURCE),
            stamp: Stamp::Version(29),
        },
        HashedSurface {
            name: "the triggers and the interrupt mask of each port of the virtual UART",
            source: Some(VIRTUAL_UART_PORT_PARAMS_SOURCE),
            stamp: Stamp::Version(29),
        },
        HashedSurface {
            name:
                "the bytes that the partition sent of a request that is not complete on each port \
                   of the virtual UART",
            source: Some(VIRTUAL_UART_REQUEST_SOURCE),
            stamp: Stamp::Version(29),
        },
        HashedSurface {
            name: "the bytes in the receive buffer of each service port of the virtual UART",
            source: Some(VIRTUAL_UART_RECEIVED_SOURCE),
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

/// The lane source of the requests that are not complete: one object for
/// each port with such a request, keyed by its index in port order.
pub(crate) const VIRTUAL_UART_REQUEST_SOURCE: u8 = 242;

/// The lane source of the receive buffers that hold bytes: one object for
/// each port with such a buffer, keyed by its index in port order.
pub(crate) const VIRTUAL_UART_RECEIVED_SOURCE: u8 = 243;

/// The virtual UART group's state:
/// - the interrupt configuration of the partition's virtual UART;
/// - the triggers and the interrupt mask of each port;
/// - the bytes of the request that is not complete on each port;
/// - the bytes in the receive buffer of each port.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UartState {
    irq: Option<VirtualUartIrq>,
    ports: BTreeMap<u64, VirtualUartPortParams>,
    requests: BTreeMap<u64, PendingRequest>,
    received: BTreeMap<u64, ReceivedBytes>,
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

    /// The bytes that the partition sent of the request that is not
    /// complete on port `port`.
    ///
    /// A port with no entry has [`PendingRequest::empty`]. A request with
    /// no byte removes the entry.
    pub fn pending_request(&self, port: u64) -> PendingRequest {
        self.requests
            .get(&port)
            .copied()
            .unwrap_or(PendingRequest::empty(port))
    }

    /// The bytes in the receive buffer of port `port`, first byte to read
    /// first.
    ///
    /// A port with no entry holds no byte. A buffer with no byte removes
    /// the entry.
    pub fn received(&self, port: u64) -> &[u8] {
        self.received
            .get(&port)
            .map_or(&[], |received| received.bytes.as_slice())
    }

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
            UartChange::RequestBytesSent(request) => {
                if request.sent == 0 {
                    self.requests.remove(&request.port);
                } else {
                    self.requests.insert(request.port, request);
                }
            }
            UartChange::ReceiveBufferSet(received) => {
                if received.bytes.is_empty() {
                    self.received.remove(&received.port);
                } else {
                    self.received.insert(received.port, received);
                }
            }
        }
    }

    /// The sync-state terms of the interrupt configuration, the ports'
    /// values, the requests that are not complete and the receive buffers.
    /// An empty state adds no term.
    pub fn sync_term(&self) -> u128 {
        let irq = self
            .irq
            .map_or(0, |irq| lanes::value_term(VIRTUAL_UART_IRQ_SOURCE, 0, &irq));
        let ports = (0u64..)
            .zip(self.ports.values())
            .fold(irq, |sum, (index, params)| {
                sum.wrapping_add(lanes::value_term(
                    VIRTUAL_UART_PORT_PARAMS_SOURCE,
                    index,
                    params,
                ))
            });
        let requests = (0u64..)
            .zip(self.requests.values())
            .fold(ports, |sum, (index, request)| {
                sum.wrapping_add(lanes::value_term(
                    VIRTUAL_UART_REQUEST_SOURCE,
                    index,
                    request,
                ))
            });
        (0u64..)
            .zip(self.received.values())
            .fold(requests, |sum, (index, received)| {
                sum.wrapping_add(lanes::value_term(
                    VIRTUAL_UART_RECEIVED_SOURCE,
                    index,
                    received,
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
    /// The call sent these bytes of a request that is not complete.
    RequestBytesSent(PendingRequest),
    /// The call left these bytes in a port's receive buffer.
    ReceiveBufferSet(ReceivedBytes),
}

impl From<UartChange> for StateChange {
    fn from(change: UartChange) -> Self {
        StateChange::Uart(Box::new(change))
    }
}
