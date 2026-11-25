//! The virtual UART of a partition: the hypercalls 162 to 166, the
//! ports of the GameOS kernel, and the two lv1 services that answer the
//! kernel's requests on its service ports.
//!
//! A port is a number up to [`VIRTUAL_UART_PORT_MAX`]. lv1 holds an
//! endpoint for each port, with a receive buffer, two triggers and an
//! interrupt mask. The endpoint's peer is the endpoint that the port
//! sends to. lv1 computes the port's interrupt status from the two
//! buffers and the triggers, and raises the partition's virtual UART
//! interrupt through one irq outlet.
//!
//! A service request is a 16-byte header and the request bytes that the
//! header counts. The service replies with a header and the reply bytes.
//! The system manager (port 2) and the dispatcher manager (port 10) each
//! have their own header form.
//!
//! The items, by group:
//! - `configure_virtual_uart_irq` (166): [`VIRTUAL_UART_IRQ_HANDLERS`],
//!   the checks from [`VIRTUAL_UART_IRQ_ADDRESS_MAX`], the r3 values from
//!   [`CONFIGURE_VIRTUAL_UART_IRQ_NO_REGION_R3`], and the kernel's outlet
//!   in [`GAME_OS_VIRTUAL_UART_IRQ_CALLS`].
//! - `get_virtual_uart_param` (165): [`VIRTUAL_UART_PARAM_HANDLERS`], the
//!   parameter ids from [`VIRTUAL_UART_PARAM_MAX`], and the kernel's
//!   ports in [`GAME_OS_VIRTUAL_UART_PORTS`].
//! - `set_virtual_uart_param` (164): [`VIRTUAL_UART_SET_PARAM_HANDLERS`],
//!   the triggers and the mask from [`VIRTUAL_UART_PARAM_TX_TRIGGER`],
//!   and the status from [`virtual_uart_status`].
//! - `read_virtual_uart` (162) and `write_virtual_uart` (163):
//!   [`VIRTUAL_UART_READ_HANDLERS`], [`VIRTUAL_UART_WRITE_HANDLERS`], and
//!   the r3 values from [`READ_VIRTUAL_UART_NO_PORT_R3`] and
//!   [`WRITE_VIRTUAL_UART_NO_PORT_R3`].
//! - The service ports: [`GAME_OS_SERVICE_PORTS`].
//! - The dispatcher manager: the header from
//!   [`DISPATCHER_HEADER_BYTES`], and the replies in
//!   [`GAME_OS_DISPATCHER_REPLIES`],
//!   [`GAME_OS_RANDOM_DISPATCHER_REQUESTS`] and
//!   [`GAME_OS_IDENTITY_DISPATCHER_REQUESTS`].
//! - The system manager: the header from
//!   [`SYSTEM_MANAGER_HEADER_BYTES`], and the replies in
//!   [`GAME_OS_SYSTEM_MANAGER_REPLIES`].
//!
//! The facts come from the 4.93 `lv1.self` and `lv2_kernel.self`, read
//! with `cellgov dev disasm`, and from console captures. The firmware
//! readings behind them are in `docs/architecture/hypervisor.md`.

use crate::hw::ppu_mmu::SMALL_PAGE_LOG2;

/// The handler of hypercall 166 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualUartIrqHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `configure_virtual_uart_irq` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 166 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const VIRTUAL_UART_IRQ_HANDLERS: &[VirtualUartIrqHandler] = &[VirtualUartIrqHandler {
    firmware: "4.93",
    handler: 0x2F2810,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn virtual_uart_irq_handler(firmware: &str) -> Option<&'static VirtualUartIrqHandler> {
    VIRTUAL_UART_IRQ_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The largest r3 that the handler looks up. It refuses an r3 above
/// 2^62 - 1, unsigned [OFW-4.93 lv1 a:0x2F289C-0x2F28A8].
pub const VIRTUAL_UART_IRQ_ADDRESS_MAX: u64 = 0x3FFF_FFFF_FFFF_FFFF;

/// The bytes of the bitmap.
///
/// The handler refuses a region with fewer than 32 bytes from r3 to its
/// end [OFW-4.93 lv1 a:0x2F2914-0x2F2924]. lv1 copies 32 bytes of its
/// shadow to the bitmap [OFW-4.93 lv1 a:0x2F1604-0x2F1620].
pub const VIRTUAL_UART_IRQ_BITMAP_BYTES: u64 = 32;

/// Whether the bitmap at `address` and its next byte are in one 4 KB page.
///
/// The handler takes the region's page size from the exponent at +160
/// [OFW-4.93 lv1 a:0x2F290C]. It refuses a bitmap whose first byte and
/// the byte after its end are in two pages
/// [OFW-4.93 lv1 a:0x2F2928-0x2F2944]. It tests the offset of r3 from the
/// region's base [OFW-4.93 lv1 a:0x2F2914]. Each PPE page size is
/// a multiple of 4 KB [CBE-Handbook p:88 s:4.2.6.6]. So for a region whose
/// base is 4 KB-aligned, such as the real mode region at 0, a bitmap that
/// passes this test passes lv1's test for each page size.
pub const fn virtual_uart_irq_bitmap_in_small_page(address: u64) -> bool {
    let page = !((1u64 << SMALL_PAGE_LOG2) - 1);
    address & page == address.wrapping_add(VIRTUAL_UART_IRQ_BITMAP_BYTES) & page
}

/// The doubleword of the virtual UART's shadow, and the bit in it, that
/// hold the interrupt of port `port`.
///
/// lv1's status function reads the port at +168 of the endpoint. It
/// takes doubleword port / 64 and bit 63 - port % 64, with bit 0 the
/// least significant [OFW-4.93 lv1 a:0x2F2214-0x2F2248]. So port 0 is the
/// most significant bit of the first doubleword.
pub const fn virtual_uart_shadow_bit(port: u8) -> (u64, u64) {
    let port = port as u64;
    (port / 64, 1 << (63 - port % 64))
}

/// The r3 of `configure_virtual_uart_irq` for an r3 that no memory region
/// of the partition holds: `LV1_NO_ENTRY` (-6), with r3 marked in the high
/// word [OFW-4.93 lv1 a:0x2F2AB8-0x2F2AC4].
///
/// As with `allocate_memory`, bit 32 marks r3. The handler returns this
/// r3 in these cases:
/// - r3 is above [`VIRTUAL_UART_IRQ_ADDRESS_MAX`];
/// - no region holds r3 [OFW-4.93 lv1 a:0x2F28F8];
/// - the region has no page array.
pub const CONFIGURE_VIRTUAL_UART_IRQ_NO_REGION_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `configure_virtual_uart_irq` for a region that fails one of
/// these checks: `LV1_TYPE_MISMATCH` (-8), with r3 marked
/// [OFW-4.93 lv1 a:0x2F2ACC-0x2F2AD8]:
/// - the region's class;
/// - the [`VIRTUAL_UART_IRQ_BITMAP_BYTES`] bytes from r3 to the region's
///   end.
pub const CONFIGURE_VIRTUAL_UART_IRQ_REGION_R3: u64 = 0x0000_0001_FFFF_FFF8;

/// The r3 of `configure_virtual_uart_irq` for a bitmap whose first byte
/// and the byte after its end are in two pages:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3 marked
/// [OFW-4.93 lv1 a:0x2F2AE0-0x2F2AEC].
pub const CONFIGURE_VIRTUAL_UART_IRQ_PAGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The r3 of `configure_virtual_uart_irq` for a partition whose virtual
/// UART already has an irq outlet: `LV1_WRONG_STATE` (-11), zero-extended
/// [OFW-4.93 lv1 a:0x2F2990-0x2F2994] [OFW-4.93 lv1 a:0x2F2A90-0x2F2A98].
pub const CONFIGURE_VIRTUAL_UART_IRQ_CONFIGURED_R3: u64 = 0x0000_0000_FFFF_FFF5;

/// The console capture of the outlet id of the 4.93 kernel's call.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the kernel addresses.
pub const VIRTUAL_UART_IRQ_OUTLET_CAPTURE: &str = "tests/micro/lv2_vuart_irq_outlet";

/// The `configure_virtual_uart_irq` call of the GameOS kernel's boot at
/// one firmware, and the outlet id that lv1 gave it on the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsVirtualUartIrqCall {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the call's `sc`.
    pub sc: u64,
    /// The outlet id in r4 of lv1's answer.
    pub outlet: u64,
}

/// The kernel's call for each firmware that a source gives the outlet id
/// for.
///
/// The partition's other objects decide the id, so no image read gives
/// it. The capture [`VIRTUAL_UART_IRQ_OUTLET_CAPTURE`] reads the id where
/// the kernel stores it after r3 = 0
/// [OFW-4.93 lv2_kernel a:0x8000000000017b10-0x8000000000017b20].
pub const GAME_OS_VIRTUAL_UART_IRQ_CALLS: &[GameOsVirtualUartIrqCall] =
    &[GameOsVirtualUartIrqCall {
        firmware: "4.93",
        sc: 0x8000_0000_0001_7b0c,
        outlet: 18,
    }];

/// The outlet id that lv1 gave the GameOS kernel's call at the `sc`
/// address `sc`.
///
/// The value is `None` when no source gives that id at the firmware
/// version `firmware`.
pub fn game_os_virtual_uart_outlet(firmware: &str, sc: u64) -> Option<u64> {
    GAME_OS_VIRTUAL_UART_IRQ_CALLS
        .iter()
        .find(|call| call.firmware == firmware && call.sc == sc)
        .map(|call| call.outlet)
}

/// The handler of hypercall 165 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualUartParamHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `get_virtual_uart_param` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 165 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const VIRTUAL_UART_PARAM_HANDLERS: &[VirtualUartParamHandler] = &[VirtualUartParamHandler {
    firmware: "4.93",
    handler: 0x2F3A68,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn virtual_uart_param_handler(firmware: &str) -> Option<&'static VirtualUartParamHandler> {
    VIRTUAL_UART_PARAM_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The largest port number. The worker refuses an r3 above 255, unsigned
/// [OFW-4.93 lv1 a:0x2F34E0] [OFW-4.93 lv1 a:0x2F34F8].
pub const VIRTUAL_UART_PORT_MAX: u64 = 0xFF;

/// The largest parameter id. The worker refuses an r4 above 7, unsigned
/// [OFW-4.93 lv1 a:0x2F35C4-0x2F35C8].
pub const VIRTUAL_UART_PARAM_MAX: u64 = 7;

/// The parameter id of the port's receive buffer size,
/// `PARAM_RX_BUF_SIZE` in Linux `drivers/ps3/ps3-vuart.c`.
///
/// The worker's entry for 3 reads +112 of the port's endpoint
/// [OFW-4.93 lv1 a:0x2F3758-0x2F3764].
pub const VIRTUAL_UART_PARAM_RX_BUFFER_SIZE: u64 = 3;

/// The parameter id of the port's transmit buffer size,
/// `PARAM_TX_BUF_SIZE` in Linux `drivers/ps3/ps3-vuart.c`.
///
/// The worker's entry for 5 reads +112 of the endpoint at +72 of the
/// port's endpoint: the receive buffer size of the peer that the port
/// sends to. It gives the value 0, with r3 = 0, when the port has no peer,
/// or when bits 2 to 31 of the word at +8 of the peer are 0
/// [OFW-4.93 lv1 a:0x2F37BC-0x2F3828] [OFW-4.93 lv1 a:0x2F399C-0x2F39A0].
/// So the value changes when lv1 links the port to another peer.
pub const VIRTUAL_UART_PARAM_TX_BUFFER_SIZE: u64 = 5;

/// The parameter id of the port's interrupt status,
/// `PARAM_INTERRUPT_STATUS` in Linux `drivers/ps3/ps3-vuart.c`.
///
/// The worker's entry for 7 locks the endpoint at +80. It then gives the
/// endpoint to the function [OFW-4.93 lv1 a:0x2F0CC4], and stores that
/// function's r3 as the value [OFW-4.93 lv1 a:0x2F3900-0x2F3954]
/// [OFW-4.93 lv1 a:0x2F367C-0x2F36A0]. That function computes the bits of
/// [`virtual_uart_status`] from the endpoint and its peer
/// [OFW-4.93 lv1 a:0x2F0D00-0x2F0D7C]. It gives 0 when the byte at +128 of
/// the endpoint is not 0 [OFW-4.93 lv1 a:0x2F0CC8-0x2F0CD4]. So the value
/// changes with the port's traffic and its triggers.
pub const VIRTUAL_UART_PARAM_INTERRUPT_STATUS: u64 = 7;

/// The r3 of `get_virtual_uart_param` for a port that lv1 does not hold:
/// `LV1_NO_ENTRY` (-6), with r3 marked in the high word
/// [OFW-4.93 lv1 a:0x2F3958-0x2F3968].
///
/// The worker returns this r3 in these cases:
/// - r3 is above [`VIRTUAL_UART_PORT_MAX`];
/// - the table holds no endpoint for r3.
pub const GET_VIRTUAL_UART_PARAM_NO_PORT_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `get_virtual_uart_param` for a parameter id above
/// [`VIRTUAL_UART_PARAM_MAX`]: `LV1_NO_ENTRY` (-6), with r4 marked in the
/// high word [OFW-4.93 lv1 a:0x2F35CC-0x2F35D8].
pub const GET_VIRTUAL_UART_PARAM_NO_PARAM_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The console capture of the buffer sizes of the 4.93 kernel's ports.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the kernel addresses.
pub const VIRTUAL_UART_PORT_CAPTURE: &str = "tests/micro/lv2_vuart_port_params";

/// A port of the GameOS partition's virtual UART at one firmware, and
/// the buffer sizes that lv1 gave the kernel's calls on the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsVirtualUartPort {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The port number in r3 of the call.
    pub port: u64,
    /// The value of [`VIRTUAL_UART_PARAM_RX_BUFFER_SIZE`].
    pub rx_buffer_size: u64,
    /// The value of [`VIRTUAL_UART_PARAM_TX_BUFFER_SIZE`].
    pub tx_buffer_size: u64,
}

/// The ports that the GameOS kernel opens, for each firmware that a source
/// gives them for, by ascending port number.
///
/// The capture [`VIRTUAL_UART_PORT_CAPTURE`] reads both sizes from the
/// kernel's records of the three ports that it opened.
pub const GAME_OS_VIRTUAL_UART_PORTS: &[GameOsVirtualUartPort] = &[
    GameOsVirtualUartPort {
        firmware: "4.93",
        port: 0,
        rx_buffer_size: 0x800,
        tx_buffer_size: 0x800,
    },
    GameOsVirtualUartPort {
        firmware: "4.93",
        port: 2,
        rx_buffer_size: 0x800,
        tx_buffer_size: 0x800,
    },
    GameOsVirtualUartPort {
        firmware: "4.93",
        port: 10,
        rx_buffer_size: 0x1000,
        tx_buffer_size: 0x1000,
    },
];

/// The GameOS partition's port `port` at the firmware version `firmware`.
///
/// The value is `None` when no source gives that port at `firmware`.
pub fn game_os_virtual_uart_port(
    firmware: &str,
    port: u64,
) -> Option<&'static GameOsVirtualUartPort> {
    GAME_OS_VIRTUAL_UART_PORTS
        .iter()
        .find(|row| row.firmware == firmware && row.port == port)
}

/// A `get_virtual_uart_param` call of the GameOS kernel's open function at
/// one firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsVirtualUartParamCall {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the call's `sc`.
    pub sc: u64,
    /// The parameter id in r4 of the call.
    pub param: u64,
}

/// The kernel's calls for the buffer sizes, for each firmware that a
/// source gives them for, by ascending `sc` address.
///
/// The capture [`VIRTUAL_UART_PORT_CAPTURE`] reads lv1's answers to these
/// calls, and to no other call. The 4.93 kernel's open function sends 165
/// with r4 = 3 [OFW-4.93 lv2_kernel a:0x8000000000017714-0x8000000000017730],
/// then with r4 = 5
/// [OFW-4.93 lv2_kernel a:0x8000000000017744-0x8000000000017750].
pub const GAME_OS_VIRTUAL_UART_PARAM_CALLS: &[GameOsVirtualUartParamCall] = &[
    GameOsVirtualUartParamCall {
        firmware: "4.93",
        sc: 0x8000_0000_0001_7730,
        param: VIRTUAL_UART_PARAM_RX_BUFFER_SIZE,
    },
    GameOsVirtualUartParamCall {
        firmware: "4.93",
        sc: 0x8000_0000_0001_7750,
        param: VIRTUAL_UART_PARAM_TX_BUFFER_SIZE,
    },
];

/// Whether the GameOS kernel's open function asks for parameter `param` at
/// the `sc` address `sc` at the firmware version `firmware`.
pub fn is_game_os_virtual_uart_param_call(firmware: &str, sc: u64, param: u64) -> bool {
    GAME_OS_VIRTUAL_UART_PARAM_CALLS
        .iter()
        .any(|call| call.firmware == firmware && call.sc == sc && call.param == param)
}

/// The handler of hypercall 164 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualUartSetParamHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `set_virtual_uart_param` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 164 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const VIRTUAL_UART_SET_PARAM_HANDLERS: &[VirtualUartSetParamHandler] =
    &[VirtualUartSetParamHandler {
        firmware: "4.93",
        handler: 0x2F4100,
    }];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn virtual_uart_set_param_handler(
    firmware: &str,
) -> Option<&'static VirtualUartSetParamHandler> {
    VIRTUAL_UART_SET_PARAM_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The parameter id of the port's transmit trigger, `PARAM_TX_TRIGGER` in
/// Linux `drivers/ps3/ps3-vuart.c`.
///
/// The worker stores r5 for this id at +136 of the endpoint
/// [OFW-4.93 lv1 a:0x2F3D0C-0x2F3D4C].
pub const VIRTUAL_UART_PARAM_TX_TRIGGER: u64 = 0;

/// The parameter id of the port's receive trigger, `PARAM_RX_TRIGGER` in
/// Linux `drivers/ps3/ps3-vuart.c`.
///
/// The worker stores r5 for this id at +144 of the endpoint
/// [OFW-4.93 lv1 a:0x2F3CFC-0x2F3D00] [OFW-4.93 lv1 a:0x2F3E58-0x2F3E94].
pub const VIRTUAL_UART_PARAM_RX_TRIGGER: u64 = 1;

/// The parameter id of the port's interrupt mask, `PARAM_INTERRUPT_MASK`
/// in Linux `drivers/ps3/ps3-vuart.c`.
///
/// The worker stores r5 for this id at +152 of the endpoint
/// [OFW-4.93 lv1 a:0x2F3DF0-0x2F3DF4] [OFW-4.93 lv1 a:0x2F3F4C-0x2F3F8C].
pub const VIRTUAL_UART_PARAM_INTERRUPT_MASK: u64 = 2;

/// The largest parameter id that `set_virtual_uart_param` sets. The
/// worker refuses an r4 above 2, unsigned
/// [OFW-4.93 lv1 a:0x2F3CFC-0x2F3D08] [OFW-4.93 lv1 a:0x2F3DF0-0x2F3DF8].
pub const VIRTUAL_UART_SET_PARAM_MAX: u64 = VIRTUAL_UART_PARAM_INTERRUPT_MASK;

/// The largest interrupt mask. The worker refuses an r5 with a bit above
/// bit 2 for the mask [OFW-4.93 lv1 a:0x2F3F40-0x2F3F48]. The status
/// bits that a mask can match are 1, 2 and 4; see [`virtual_uart_status`].
pub const VIRTUAL_UART_INTERRUPT_MASK_MAX: u64 = 7;

/// The status bit of a port whose peer's receive buffer holds no more
/// bytes than the transmit trigger: `INTERRUPT_MASK_TX` in Linux
/// `drivers/ps3/ps3-vuart.c`.
pub const VIRTUAL_UART_STATUS_TX: u64 = 1;

/// The status bit of a port whose receive buffer has no more free bytes
/// than the receive trigger: `INTERRUPT_MASK_RX` in Linux
/// `drivers/ps3/ps3-vuart.c`.
pub const VIRTUAL_UART_STATUS_RX: u64 = 2;

/// The status bit of a port with no peer that has a reference:
/// `INTERRUPT_MASK_DISCONNECT` in Linux `drivers/ps3/ps3-vuart.c`.
pub const VIRTUAL_UART_STATUS_DISCONNECT: u64 = 4;

/// The status bit of a port with a peer that has a reference. No mask
/// that lv1 accepts holds this bit.
pub const VIRTUAL_UART_STATUS_CONNECTED: u64 = 8;

/// The fields of a port's endpoint, other than the triggers, that lv1's
/// status function reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualUartTraffic {
    /// The size of the receive buffer, at +112 of the endpoint.
    pub buffer_size: u64,
    /// The bytes that the receive buffer holds, at +120 of the endpoint.
    pub buffer_count: u64,
    /// The bytes that the peer's receive buffer holds, at +120 of the
    /// peer. The value is `None` when the port has no peer with a
    /// reference.
    pub peer_count: Option<u64>,
}

/// The interrupt status that lv1's status function computes for a port
/// with `traffic` and the triggers `tx_trigger` and `rx_trigger`.
///
/// The function [OFW-4.93 lv1 a:0x2F20E4] sets these bits:
/// - [`VIRTUAL_UART_STATUS_DISCONNECT`] when the peer at +72 is 0, or the
///   peer's reference count is 0 [OFW-4.93 lv1 a:0x2F21B0-0x2F21E8];
/// - [`VIRTUAL_UART_STATUS_CONNECTED`] for a peer with a reference, and
///   [`VIRTUAL_UART_STATUS_TX`] too when the peer's count is not above
///   the transmit trigger, unsigned [OFW-4.93 lv1 a:0x2F21F4-0x2F2204]
///   [OFW-4.93 lv1 a:0x2F2380-0x2F2384];
/// - [`VIRTUAL_UART_STATUS_RX`] when the buffer size less the count is not
///   above the receive trigger, unsigned
///   [OFW-4.93 lv1 a:0x2F2208-0x2F2210] [OFW-4.93 lv1 a:0x2F2388-0x2F238C]
///   [OFW-4.93 lv1 a:0x2F2348].
pub const fn virtual_uart_status(
    traffic: VirtualUartTraffic,
    tx_trigger: u64,
    rx_trigger: u64,
) -> u64 {
    let mut status = match traffic.peer_count {
        None => VIRTUAL_UART_STATUS_DISCONNECT,
        Some(peer_count) if peer_count <= tx_trigger => {
            VIRTUAL_UART_STATUS_CONNECTED | VIRTUAL_UART_STATUS_TX
        }
        Some(_) => VIRTUAL_UART_STATUS_CONNECTED,
    };
    if traffic.buffer_size.wrapping_sub(traffic.buffer_count) <= rx_trigger {
        status |= VIRTUAL_UART_STATUS_RX;
    }
    status
}

/// The r3 of `set_virtual_uart_param` for a port that lv1 does not hold:
/// `LV1_NO_ENTRY` (-6), with r3 marked in the high word
/// [OFW-4.93 lv1 a:0x2F4038-0x2F4044].
///
/// The worker returns this r3 in these cases:
/// - r3 is above [`VIRTUAL_UART_PORT_MAX`];
/// - the table holds no endpoint for r3.
pub const SET_VIRTUAL_UART_PARAM_NO_PORT_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `set_virtual_uart_param` for a parameter id above
/// [`VIRTUAL_UART_SET_PARAM_MAX`]: `LV1_NO_ENTRY` (-6), with r4 marked in
/// the high word [OFW-4.93 lv1 a:0x2F3DF8-0x2F3E04].
pub const SET_VIRTUAL_UART_PARAM_NO_PARAM_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The r3 of `set_virtual_uart_param` for a mask above
/// [`VIRTUAL_UART_INTERRUPT_MASK_MAX`]: `LV1_ILLEGAL_PARAMETER_VALUE`
/// (-17), with r5 marked in the high word
/// [OFW-4.93 lv1 a:0x2F40EC-0x2F40F8].
pub const SET_VIRTUAL_UART_PARAM_MASK_R3: u64 = 0x0000_0004_FFFF_FFEF;

/// The console capture of the interrupt mask that the 4.93 kernel keeps
/// for each of its ports.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the kernel addresses.
pub const VIRTUAL_UART_PORT_MASK_CAPTURE: &str = "tests/micro/lv2_vuart_port_mask";

/// The handler of hypercall 162 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualUartReadHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `read_virtual_uart` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 162 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const VIRTUAL_UART_READ_HANDLERS: &[VirtualUartReadHandler] = &[VirtualUartReadHandler {
    firmware: "4.93",
    handler: 0x2F429C,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn virtual_uart_read_handler(firmware: &str) -> Option<&'static VirtualUartReadHandler> {
    VIRTUAL_UART_READ_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The largest buffer address that `read_virtual_uart` looks up. The
/// handler refuses an r4 above 2^62 - 1, unsigned
/// [OFW-4.93 lv1 a:0x2F4344-0x2F4350].
pub const VIRTUAL_UART_READ_ADDRESS_MAX: u64 = 0x3FFF_FFFF_FFFF_FFFF;

/// The r3 of `read_virtual_uart` for a port that lv1 does not hold:
/// `LV1_NO_ENTRY` (-6), with r3 marked in the high word
/// [OFW-4.93 lv1 a:0x2F4604-0x2F4610].
///
/// The handler returns this r3 in these cases:
/// - r3 is above [`VIRTUAL_UART_PORT_MAX`];
/// - the table holds no endpoint for r3;
/// - the read function returns 7, because the byte at +128 of the
///   endpoint is not 0 [OFW-4.93 lv1 a:0x2F45D8-0x2F45DC].
pub const READ_VIRTUAL_UART_NO_PORT_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `read_virtual_uart` for a buffer address that no memory
/// region holds: `LV1_NO_ENTRY` (-6), with r4 marked in the high word
/// [OFW-4.93 lv1 a:0x2F4618-0x2F4624].
///
/// The handler returns this r3 in these cases:
/// - r4 is above [`VIRTUAL_UART_READ_ADDRESS_MAX`];
/// - no region holds r4;
/// - the region has no page array.
pub const READ_VIRTUAL_UART_NO_REGION_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The r3 of `read_virtual_uart` for a region that fails one of these
/// checks: `LV1_TYPE_MISMATCH` (-8), with r4 marked in the high word
/// [OFW-4.93 lv1 a:0x2F462C-0x2F4638]:
/// - the region's class;
/// - the r5 bytes from r4 to the region's end.
pub const READ_VIRTUAL_UART_REGION_R3: u64 = 0x0000_0002_FFFF_FFF8;

/// The handler of hypercall 163 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualUartWriteHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `write_virtual_uart` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 163 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const VIRTUAL_UART_WRITE_HANDLERS: &[VirtualUartWriteHandler] = &[VirtualUartWriteHandler {
    firmware: "4.93",
    handler: 0x2F2AF4,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn virtual_uart_write_handler(firmware: &str) -> Option<&'static VirtualUartWriteHandler> {
    VIRTUAL_UART_WRITE_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The largest buffer address that `write_virtual_uart` looks up. The
/// handler refuses an r4 above 2^62 - 1, unsigned
/// [OFW-4.93 lv1 a:0x2F2B98-0x2F2BA4].
pub const VIRTUAL_UART_WRITE_ADDRESS_MAX: u64 = 0x3FFF_FFFF_FFFF_FFFF;

/// The r3 of `write_virtual_uart` for a port that lv1 does not hold:
/// `LV1_NO_ENTRY` (-6), with r3 marked in the high word
/// [OFW-4.93 lv1 a:0x2F2EC8-0x2F2ED4].
///
/// The handler returns this r3 in these cases:
/// - r3 is above [`VIRTUAL_UART_PORT_MAX`];
/// - the table holds no endpoint for r3;
/// - the write function returns 7 before it writes a byte
///   [OFW-4.93 lv1 a:0x2F2E9C-0x2F2EA0].
pub const WRITE_VIRTUAL_UART_NO_PORT_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `write_virtual_uart` for a buffer address that no memory
/// region holds: `LV1_NO_ENTRY` (-6), with r4 marked in the high word
/// [OFW-4.93 lv1 a:0x2F2EF0-0x2F2EFC].
///
/// The handler returns this r3 in these cases:
/// - r4 is above [`VIRTUAL_UART_WRITE_ADDRESS_MAX`];
/// - no region holds r4 [OFW-4.93 lv1 a:0x2F2C94-0x2F2C98];
/// - the region has no page array [OFW-4.93 lv1 a:0x2F2CC8-0x2F2CCC].
pub const WRITE_VIRTUAL_UART_NO_REGION_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The r3 of `write_virtual_uart` for a region that fails one of these
/// checks: `LV1_TYPE_MISMATCH` (-8), with r4 marked in the high word
/// [OFW-4.93 lv1 a:0x2F2F10-0x2F2F1C]:
/// - the region's class;
/// - the r5 bytes from r4 to the region's end.
pub const WRITE_VIRTUAL_UART_REGION_R3: u64 = 0x0000_0002_FFFF_FFF8;

/// The r3 of `write_virtual_uart` for a port with no peer that has a
/// reference: `LV1_WRONG_STATE` (-11), with r3 marked in the high word
/// [OFW-4.93 lv1 a:0x2F2EDC-0x2F2EE8].
pub const WRITE_VIRTUAL_UART_NO_PEER_R3: u64 = 0x0000_0001_FFFF_FFF5;

/// An lv1 service that holds the peer of a port of the GameOS partition's
/// virtual UART.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualUartService {
    /// The system manager, `sysmgr_ss.fself` of lv1's process archive.
    SystemManager,
    /// The dispatcher manager of `ss_server3.fself` of lv1's process
    /// archive.
    DispatcherManager,
}

/// A port of the GameOS partition's virtual UART that an lv1 service
/// constructs at one firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsServicePort {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The port number.
    pub port: u64,
    /// The service that holds the port's peer.
    pub service: VirtualUartService,
    /// The bytes of the buffer that the service gives the port.
    pub buffer_bytes: u64,
}

/// The service ports for each firmware whose lv1 image shows them, by
/// ascending port number.
///
/// Each service gives process system call 0x10084 the port and the buffer
/// size:
/// - the 4.93 system manager, port 2 and 2048 bytes
///   [OFW-4.93 lv1 a:0x44CF04-0x44CF0C] [OFW-4.93 lv1 a:0x44CF24];
/// - the 4.93 dispatcher manager, the port that the secure loader code
///   gives it and 4096 bytes [OFW-4.93 lv1 a:0x5BF1F8-0x5BF1FC]. That port
///   is 10 [OFW-4.93 lv1 a:0x5B7E7C-0x5B7E80].
///
/// Each port's receive buffer holds no byte at the GameOS kernel's first
/// read.
pub const GAME_OS_SERVICE_PORTS: &[GameOsServicePort] = &[
    GameOsServicePort {
        firmware: "4.93",
        port: 2,
        service: VirtualUartService::SystemManager,
        buffer_bytes: 0x800,
    },
    GameOsServicePort {
        firmware: "4.93",
        port: 10,
        service: VirtualUartService::DispatcherManager,
        buffer_bytes: 0x1000,
    },
];

/// The service port `port` at the firmware version `firmware`, or `None`
/// when no lv1 image read shows a service that holds its peer.
pub fn game_os_service_port(firmware: &str, port: u64) -> Option<&'static GameOsServicePort> {
    GAME_OS_SERVICE_PORTS
        .iter()
        .find(|row| row.firmware == firmware && row.port == port)
}

/// The bytes of the header of a request that the dispatcher manager reads
/// from its port first [OFW-4.93 lv1 a:0x5BD8B8-0x5BD8BC].
pub const DISPATCHER_HEADER_BYTES: u64 = 16;

/// The service id that the request header `header` gives: its big-endian
/// word at +4.
///
/// The port function looks up that word [OFW-4.93 lv1 a:0x5BD910-0x5BD920].
pub const fn dispatcher_header_service(header: &[u8; DISPATCHER_HEADER_BYTES as usize]) -> u32 {
    u32::from_be_bytes([header[4], header[5], header[6], header[7]])
}

/// The count of request bytes after the request header `header`: its
/// big-endian word at +8.
///
/// The port function gives that word to the discard function
/// [OFW-4.93 lv1 a:0x5BD934-0x5BD93C], and to the read of the request
/// bytes [OFW-4.93 lv1 a:0x5BDA18-0x5BDA28].
pub const fn dispatcher_header_request_bytes(
    header: &[u8; DISPATCHER_HEADER_BYTES as usize],
) -> u32 {
    u32::from_be_bytes([header[8], header[9], header[10], header[11]])
}

/// The service id of the 4.93 GameOS kernel's first request to the
/// dispatcher manager.
///
/// The constructor of the kernel's request object stores it at +16
/// [OFW-4.93 lv2_kernel a:0x8000000000229950]
/// [OFW-4.93 lv2_kernel a:0x800000000022995c].
pub const GAME_OS_FIRST_DISPATCHER_SERVICE: u32 = 0x19000;

/// The request id of the 4.93 GameOS kernel's first request to the
/// dispatcher manager.
///
/// The constructor of the kernel's request object stores it at +8
/// [OFW-4.93 lv2_kernel a:0x800000000022994c]
/// [OFW-4.93 lv2_kernel a:0x8000000000229958]. In CellGov's boot, the
/// request bytes that the kernel sends start with this id, as a
/// big-endian doubleword.
pub const GAME_OS_FIRST_DISPATCHER_REQUEST: u64 = 0x19002;

/// The tag of the request header `header`: its big-endian word at +0.
///
/// The port function keeps the tag for the reply
/// [OFW-4.93 lv1 a:0x5BDC0C] [OFW-4.93 lv1 a:0x5BC9A4].
pub const fn dispatcher_header_tag(header: &[u8; DISPATCHER_HEADER_BYTES as usize]) -> u32 {
    u32::from_be_bytes([header[0], header[1], header[2], header[3]])
}

/// The count of reply bytes that the request header `header` gives: its
/// big-endian word at +12.
///
/// The port function keeps the count for the reply
/// [OFW-4.93 lv1 a:0x5BDC08] [OFW-4.93 lv1 a:0x5BC990].
pub const fn dispatcher_header_reply_bytes(header: &[u8; DISPATCHER_HEADER_BYTES as usize]) -> u32 {
    u32::from_be_bytes([header[12], header[13], header[14], header[15]])
}

/// The header that the dispatcher manager writes to its port before a
/// service's reply to a request with the tag `tag` and `reply_bytes`
/// reply bytes.
///
/// The service function puts the tag in the word at +0, 0 in the words at
/// +4 and +8, and the count in the word at +12
/// [OFW-4.93 lv1 a:0x5BD4FC-0x5BD520]. It takes that path only when its
/// receive call returns 0 in r31 [OFW-4.93 lv1 a:0x5BD464-0x5BD474].
pub const fn dispatcher_reply_header(
    tag: u32,
    reply_bytes: u32,
) -> [u8; DISPATCHER_HEADER_BYTES as usize] {
    let t = tag.to_be_bytes();
    let n = reply_bytes.to_be_bytes();
    [
        t[0], t[1], t[2], t[3], 0, 0, 0, 0, 0, 0, 0, 0, n[0], n[1], n[2], n[3],
    ]
}

/// The offset of the 4-byte status word in the reply bytes of a request
/// that `ss_server2.fself` answers [OFW-4.93 lv1 a:0x58D684-0x58D6A8].
pub const DISPATCHER_REPLY_STATUS_OFFSET: usize = 16;

/// The offset of the 16 data bytes in the reply bytes of request 0x19002
/// [OFW-4.93 lv1 a:0x58D59C] [OFW-4.93 lv1 a:0x56D234-0x56D248].
pub const DISPATCHER_REPLY_DATA_OFFSET: usize = 40;

/// The bytes of the data field of the reply to request 0x19002
/// [OFW-4.93 lv1 a:0x56D244].
pub const DISPATCHER_REPLY_DATA_BYTES: usize = 16;

/// The console capture of the reply that the 4.93 kernel kept from its
/// first dispatcher manager request.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the kernel addresses.
pub const DISPATCHER_REPLY_CAPTURE: &str = "tests/micro/lv2_aim_device_type";

/// The reply of a service of lv1 to one request of the GameOS kernel, at
/// one firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsDispatcherReply {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The service id that the request header gives.
    pub service: u32,
    /// The request id that the request bytes start with.
    pub request: u64,
    /// The count of request bytes and the count of reply bytes that the
    /// request header gives.
    pub bytes: u32,
    /// The status word at [`DISPATCHER_REPLY_STATUS_OFFSET`].
    pub status: u32,
    /// The data bytes at [`DISPATCHER_REPLY_DATA_OFFSET`].
    pub data: [u8; DISPATCHER_REPLY_DATA_BYTES],
}

/// Each reply that a source gives.
///
/// The 4.93 kernel's first request has 0x38 request bytes and 0x38 reply
/// bytes; see [`GAME_OS_FIRST_DISPATCHER_REQUEST`]. The capture
/// [`DISPATCHER_REPLY_CAPTURE`] gives its status and data bytes. The data
/// bytes are the value of the reference console's class. No image read
/// gives them.
pub const GAME_OS_DISPATCHER_REPLIES: &[GameOsDispatcherReply] = &[GameOsDispatcherReply {
    firmware: "4.93",
    service: GAME_OS_FIRST_DISPATCHER_SERVICE,
    request: GAME_OS_FIRST_DISPATCHER_REQUEST,
    bytes: 0x38,
    status: 0,
    data: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x84],
}];

/// The reply to request `request` of service `service` at the firmware
/// version `firmware`, or `None` when no source gives it.
pub fn game_os_dispatcher_reply(
    firmware: &str,
    service: u32,
    request: u64,
) -> Option<&'static GameOsDispatcherReply> {
    GAME_OS_DISPATCHER_REPLIES
        .iter()
        .find(|row| row.firmware == firmware && row.service == service && row.request == request)
}

/// The bytes of random data in the reply to a request of
/// [`GAME_OS_RANDOM_DISPATCHER_REQUESTS`], at
/// [`DISPATCHER_REPLY_DATA_OFFSET`]
/// [OFW-4.93 lv2_kernel a:0x8000000000227fe0-0x80000000002280dc]
/// [OFW-4.93 lv1 a:0x50B54C].
pub const DISPATCHER_RANDOM_DATA_BYTES: usize = 24;

/// A request of the GameOS kernel whose reply holds random bytes that lv1
/// draws, at one firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsRandomDispatcherRequest {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The service id that the request header gives.
    pub service: u32,
    /// The request id that the request bytes start with.
    pub request: u64,
    /// The count of request bytes and the count of reply bytes that the
    /// request header gives.
    pub bytes: u32,
    /// The status word at [`DISPATCHER_REPLY_STATUS_OFFSET`] of a reply in
    /// which lv1 writes the random data bytes.
    pub status: u32,
}

/// Each request whose reply data an lv1 image read shows to be random.
///
/// The 4.93 kernel's class gives 0x40 as both counts
/// [OFW-4.93 lv2_kernel a:0x8000000000228178]. It sets the request id
/// 0x5003 and the service id 0x5000
/// [OFW-4.93 lv2_kernel a:0x8000000000227fb0-0x8000000000227fc0]. The
/// lv1 worker returns 0 when it writes the random data bytes, and the
/// handler stores that value as the status
/// [OFW-4.93 lv1 a:0x50B534-0x50B558] [OFW-4.93 lv1 a:0x508D7C-0x508D84].
pub const GAME_OS_RANDOM_DISPATCHER_REQUESTS: &[GameOsRandomDispatcherRequest] =
    &[GameOsRandomDispatcherRequest {
        firmware: "4.93",
        service: 0x5000,
        request: 0x5003,
        bytes: 0x40,
        status: 0,
    }];

/// The request `request` of service `service` at the firmware version
/// `firmware`, or `None` when no image read shows its reply data to be
/// random.
pub fn game_os_random_dispatcher_request(
    firmware: &str,
    service: u32,
    request: u64,
) -> Option<&'static GameOsRandomDispatcherRequest> {
    GAME_OS_RANDOM_DISPATCHER_REQUESTS
        .iter()
        .find(|row| row.firmware == firmware && row.service == service && row.request == request)
}

/// The bytes of identity data in the reply to a request of
/// [`GAME_OS_IDENTITY_DISPATCHER_REQUESTS`], at
/// [`DISPATCHER_REPLY_DATA_OFFSET`].
///
/// The 4.93 kernel's unmarshal copies the 8 reply bytes at +40 to its
/// request object
/// [OFW-4.93 lv2_kernel a:0x8000000000229e30-0x8000000000229e78]. The lv1
/// worker copies the first 8 bytes of the module's result to its field
/// [OFW-4.93 lv1 a:0x56D964-0x56D970] [OFW-4.93 lv1 a:0x56DD6C-0x56DD78].
pub const DISPATCHER_IDENTITY_DATA_BYTES: usize = 8;

/// A request of the GameOS kernel whose reply data an isolated SPU module
/// of lv1 derives from data of the console, at one firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsIdentityDispatcherRequest {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The service id that the request header gives.
    pub service: u32,
    /// The request id that the request bytes start with.
    pub request: u64,
    /// The count of request bytes and the count of reply bytes that the
    /// request header gives.
    pub bytes: u32,
}

/// Each request whose reply data an lv1 image read shows to come from
/// data of the console.
///
/// At 4.93, the kernel's class for request 0x19004 puts the request id at
/// +8 and the service id 0x19000 at +16
/// [OFW-4.93 lv2_kernel a:0x800000000022989c-0x80000000002298ac]. Its
/// size function gives 0x30 as both counts
/// [OFW-4.93 lv2_kernel a:0x8000000000229f34].
///
/// An isolated SPU module of lv1 derives the data bytes from data of the
/// console, so no image read gives them. CellGov may serve
/// identity-derived bytes only from one committed synthetic console
/// identity. It commits no such identity yet.
pub const GAME_OS_IDENTITY_DISPATCHER_REQUESTS: &[GameOsIdentityDispatcherRequest] =
    &[GameOsIdentityDispatcherRequest {
        firmware: "4.93",
        service: 0x19000,
        request: 0x19004,
        bytes: 0x30,
    }];

/// The request `request` of service `service` at the firmware version
/// `firmware`, or `None` when no image read shows its reply data to come
/// from data of the console.
pub fn game_os_identity_dispatcher_request(
    firmware: &str,
    service: u32,
    request: u64,
) -> Option<&'static GameOsIdentityDispatcherRequest> {
    GAME_OS_IDENTITY_DISPATCHER_REQUESTS
        .iter()
        .find(|row| row.firmware == firmware && row.service == service && row.request == request)
}

/// The bytes of the header of a request that the system manager reads
/// from its port first, with one read
/// [OFW-4.93 lv1 a:0x44CD84-0x44CD94].
pub const SYSTEM_MANAGER_HEADER_BYTES: u64 = 16;

/// The count of payload bytes that the system manager's request header
/// `header` gives: its big-endian word at +4.
///
/// The receive function loads that word for the payload read
/// [OFW-4.93 lv1 a:0x44CDB8]. Linux `struct ps3_sys_manager_header` names
/// it `payload_size`.
pub const fn system_manager_header_payload_bytes(
    header: &[u8; SYSTEM_MANAGER_HEADER_BYTES as usize],
) -> u32 {
    u32::from_be_bytes([header[4], header[5], header[6], header[7]])
}

/// The service id that the system manager's request header `header`
/// gives: its big-endian halfword at +8.
///
/// The receive function loads that halfword for its log call
/// [OFW-4.93 lv1 a:0x44CD9C]. Linux `struct ps3_sys_manager_header` names
/// it `service_id`.
pub const fn system_manager_header_service(
    header: &[u8; SYSTEM_MANAGER_HEADER_BYTES as usize],
) -> u16 {
    u16::from_be_bytes([header[8], header[9]])
}

/// The tag of the system manager's request header `header`: its
/// big-endian word at +12.
///
/// The receive function gives that word to the request handler in r4
/// [OFW-4.93 lv1 a:0x44CE30]. Linux `struct ps3_sys_manager_header` names
/// it `request_tag`.
pub const fn system_manager_header_tag(header: &[u8; SYSTEM_MANAGER_HEADER_BYTES as usize]) -> u32 {
    u32::from_be_bytes([header[12], header[13], header[14], header[15]])
}

/// The largest count of payload bytes that the system manager's receive
/// function reads after one header [OFW-4.93 lv1 a:0x44CDBC-0x44CDC4].
pub const SYSTEM_MANAGER_PAYLOAD_MAX: u32 = 2048;

/// The count of payload bytes that the system manager's receive function
/// reads after a header that counts `payload_bytes`.
///
/// The function compares the count as a signed word
/// [OFW-4.93 lv1 a:0x44CDB8-0x44CDD8]. The value is one of these:
/// - `None` for 0: the function ends with no handler and no reply;
/// - 0 for a negative count: the read loop reads no byte, and the handler
///   runs [OFW-4.93 lv1 a:0x44CDFC-0x44CE10];
/// - [`SYSTEM_MANAGER_PAYLOAD_MAX`] for a count above it. The port keeps
///   the other bytes, and the next call reads them as a header;
/// - the count for each other value.
pub const fn system_manager_payload_read(payload_bytes: u32) -> Option<u32> {
    let count = payload_bytes as i32;
    if count == 0 {
        None
    } else if count < 0 {
        Some(0)
    } else if payload_bytes > SYSTEM_MANAGER_PAYLOAD_MAX {
        Some(SYSTEM_MANAGER_PAYLOAD_MAX)
    } else {
        Some(payload_bytes)
    }
}

/// The largest index of the system manager's request handler table. The
/// table holds 62 entries [OFW-4.93 lv1 a:0x46FD60-0x46FF4C].
pub const SYSTEM_MANAGER_HANDLER_INDEX_MAX: u8 = 0x3D;

/// The index that the system manager's receive function takes for a
/// service id whose low byte is above [`SYSTEM_MANAGER_HANDLER_INDEX_MAX`]
/// [OFW-4.93 lv1 a:0x44CE18-0x44CE20].
///
/// Linux names service id 6 `PS3_SM_SERVICE_ID_REQUEST_ERROR`.
pub const SYSTEM_MANAGER_REQUEST_ERROR_INDEX: u8 = 6;

/// The index of the request handler that the system manager's receive
/// function calls for the service id `service`.
///
/// The function loads only the low byte of the service id, header byte 9
/// [OFW-4.93 lv1 a:0x44CE14]. A byte above
/// [`SYSTEM_MANAGER_HANDLER_INDEX_MAX`] becomes
/// [`SYSTEM_MANAGER_REQUEST_ERROR_INDEX`] [OFW-4.93 lv1 a:0x44CE18-0x44CE20].
pub const fn system_manager_handler_index(service: u16) -> u8 {
    let low = service.to_be_bytes()[1];
    if low > SYSTEM_MANAGER_HANDLER_INDEX_MAX {
        SYSTEM_MANAGER_REQUEST_ERROR_INDEX
    } else {
        low
    }
}

/// The service id of the 4.93 GameOS kernel's first request to the
/// system manager in CellGov's boot.
///
/// The kernel's function stores it at +8 of the header
/// [OFW-4.93 lv2_kernel a:0x80000000002646fc]
/// [OFW-4.93 lv2_kernel a:0x800000000026470c]. Entry 9 of the receive
/// function's table holds its handler; see
/// [`GAME_OS_SYSTEM_MANAGER_REPLIES`].
pub const GAME_OS_FIRST_SYSTEM_MANAGER_SERVICE: u16 = 9;

/// The header that the system manager writes to its port before a reply
/// of the service id `service`, with `payload_bytes` payload bytes and
/// the tag `tag`.
///
/// The send function gives the header builder the low byte of its service
/// id, its tag, and its payload count plus its r7
/// [OFW-4.93 lv1 a:0x44C4CC-0x44C510]. The builder sets these fields
/// [OFW-4.93 lv1 a:0x44BA78-0x44BAA8]:
/// - the version 1 at +0 and the size 16 at +1;
/// - the count as a word at +4;
/// - the service id as a halfword at +8;
/// - the tag as a word at +12;
/// - 0 in each other byte.
///
/// The send function then writes the header and the payload with one
/// write [OFW-4.93 lv1 a:0x44C514-0x44C558].
pub const fn system_manager_reply_header(
    service: u8,
    payload_bytes: u32,
    tag: u32,
) -> [u8; SYSTEM_MANAGER_HEADER_BYTES as usize] {
    let n = payload_bytes.to_be_bytes();
    let t = tag.to_be_bytes();
    [
        1, 0x10, 0, 0, n[0], n[1], n[2], n[3], 0, service, 0, 0, t[0], t[1], t[2], t[3],
    ]
}

/// The offset of the payload byte that the handler of service id 9
/// branches on [OFW-4.93 lv1 a:0x44DEA0].
pub const SYSTEM_MANAGER_SELECTOR_OFFSET: usize = 1;

/// The console capture of the reply fields that the 4.93 kernel kept from
/// the system manager's replies of service ids 0x0B and 0x0A.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the kernel addresses.
pub const SYSTEM_MANAGER_REPLY_CAPTURE: &str = "tests/micro/lv2_system_manager_reply";

/// The most bytes that the system manager's init keeps for the reply of
/// service id 0x0A.
///
/// The init sets the count to byte 5 of the block at vaddr 0xC000D35B
/// only when that byte is not above 128 [OFW-4.93 lv1 a:0x44CFE0]
/// [OFW-4.93 lv1 a:0x4527D0-0x452800].
pub const SYSTEM_MANAGER_INIT_STORED_MAX: u64 = 128;

/// The system manager's reply to one request of the GameOS kernel, at one
/// firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsSystemManagerReply {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The index of the request handler; see
    /// [`system_manager_handler_index`].
    pub handler: u8,
    /// The payload byte at [`SYSTEM_MANAGER_SELECTOR_OFFSET`].
    pub selector: u8,
    /// The service id of the reply.
    pub reply_service: u8,
    /// The payload bytes of the reply.
    pub payload: &'static [u8],
    /// The bytes that the handler writes after the payload.
    pub trailing: &'static [u8],
}

impl GameOsSystemManagerReply {
    /// The payload count in the reply's header.
    ///
    /// The send function adds its r7 to its payload count
    /// [OFW-4.93 lv1 a:0x44C4CC]. A handler that writes bytes after the
    /// payload gives their count in r7 [OFW-4.93 lv1 a:0x44DF50].
    pub const fn header_count(&self) -> usize {
        self.payload.len() + self.trailing.len()
    }

    /// The bytes that the system manager writes to its port for this reply
    /// to a request with the tag `tag`, or `None` when the header count
    /// does not fit in a word.
    ///
    /// The bytes are the header of [`system_manager_reply_header`], the
    /// payload, and then the trailing bytes. The handler writes the
    /// trailing bytes after the send function returns
    /// [OFW-4.93 lv1 a:0x44DF64-0x44DFC8].
    pub fn written_bytes(&self, tag: u32) -> Option<Vec<u8>> {
        let count = u32::try_from(self.header_count()).ok()?;
        let head = system_manager_reply_header(self.reply_service, count, tag);
        Some([&head[..], self.payload, self.trailing].concat())
    }
}

/// Each system manager reply that a source gives.
///
/// The handler of service id 9 replies to payload byte 1 = 2 with service
/// id 0x0B, its tag and 16 payload bytes
/// [OFW-4.93 lv1 a:0x44DEA8] [OFW-4.93 lv1 a:0x44DF10-0x44DF24]. Its r7
/// is 0, so the header counts 16. The capture
/// [`SYSTEM_MANAGER_REPLY_CAPTURE`] gives bytes 1, 2 and 4 to 15.
///
/// The handler replies to payload byte 1 = 1 with service id 0x0A, its
/// tag and 8 payload bytes [OFW-4.93 lv1 a:0x44DF2C-0x44DF60]. It then
/// writes the bytes that the system manager keeps, which count 0 at
/// CellGov's boot.
pub const GAME_OS_SYSTEM_MANAGER_REPLIES: &[GameOsSystemManagerReply] = &[
    GameOsSystemManagerReply {
        firmware: "4.93",
        handler: 9,
        selector: 2,
        reply_service: 0x0B,
        payload: &[2, 1, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 7],
        trailing: &[],
    },
    GameOsSystemManagerReply {
        firmware: "4.93",
        handler: 9,
        selector: 1,
        reply_service: 0x0A,
        payload: &[1, 0, 0, 0, 0, 0, 0, 0],
        trailing: &[],
    },
];

/// The reply to a request of service id `service` whose payload byte at
/// [`SYSTEM_MANAGER_SELECTOR_OFFSET`] is `selector`, at the firmware
/// version `firmware`, or `None` when no source gives it.
///
/// The receive function selects the handler by
/// [`system_manager_handler_index`].
pub fn game_os_system_manager_reply(
    firmware: &str,
    service: u16,
    selector: u8,
) -> Option<&'static GameOsSystemManagerReply> {
    let handler = system_manager_handler_index(service);
    GAME_OS_SYSTEM_MANAGER_REPLIES
        .iter()
        .find(|row| row.firmware == firmware && row.handler == handler && row.selector == selector)
}

#[cfg(test)]
#[path = "tests/uart_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/uart_console_tests.rs"]
mod console_tests;

#[cfg(test)]
#[path = "tests/uart_param_tests.rs"]
mod param_tests;

#[cfg(test)]
#[path = "tests/uart_param_console_tests.rs"]
mod param_console_tests;

#[cfg(test)]
#[path = "tests/uart_set_param_tests.rs"]
mod set_param_tests;

#[cfg(test)]
#[path = "tests/uart_set_param_console_tests.rs"]
mod set_param_console_tests;

#[cfg(test)]
#[path = "tests/uart_status_tests.rs"]
mod status_tests;

#[cfg(test)]
#[path = "tests/uart_read_tests.rs"]
mod read_tests;

#[cfg(test)]
#[path = "tests/uart_service_port_tests.rs"]
mod service_port_tests;

#[cfg(test)]
#[path = "tests/uart_write_tests.rs"]
mod write_tests;

#[cfg(test)]
#[path = "tests/uart_reply_tests.rs"]
mod reply_tests;

#[cfg(test)]
#[path = "tests/uart_reply_console_tests.rs"]
mod reply_console_tests;

#[cfg(test)]
#[path = "tests/uart_random_request_tests.rs"]
mod random_request_tests;

#[cfg(test)]
#[path = "tests/uart_identity_request_tests.rs"]
mod identity_request_tests;

#[cfg(test)]
#[path = "tests/uart_shadow_tests.rs"]
mod shadow_tests;

#[cfg(test)]
#[path = "tests/uart_system_manager_tests.rs"]
mod system_manager_tests;

#[cfg(test)]
#[path = "tests/uart_system_manager_reply_tests.rs"]
mod system_manager_reply_tests;

#[cfg(test)]
#[path = "tests/uart_system_manager_reply_console_tests.rs"]
mod system_manager_reply_console_tests;

#[cfg(test)]
#[path = "tests/uart_system_manager_stored_reply_tests.rs"]
mod system_manager_stored_reply_tests;

#[cfg(test)]
#[path = "tests/uart_system_manager_stored_reply_console_tests.rs"]
mod system_manager_stored_reply_console_tests;
