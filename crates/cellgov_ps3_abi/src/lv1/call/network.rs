//! The numbers of the network device hypercalls.

/// `net_set_interrupt_status_indicator`: four inputs and no output.
/// - r3 is the id of a bus.
/// - r4 is the id of a device on that bus.
/// - r5 is the logical partition address of the device's interrupt
///   status doubleword.
/// - r6 is a fourth input, which must be 0.
///
/// The lv1 handler [OFW-4.93 lv1 a:0x308204] reads the inputs in this
/// form; see [`crate::lv1::network`].
pub const NET_SET_INTERRUPT_STATUS_INDICATOR: u64 = 191;

/// `net_control`: six inputs and two outputs.
/// - r3 is the id of a bus.
/// - r4 is the id of a device on that bus.
/// - r5 is the request code.
/// - r6 to r8 are the request's inputs.
/// - r4 and r5 return the request's outputs.
///
/// The lv1 handler [OFW-4.93 lv1 a:0x307D2C] reads the inputs in this
/// form; see [`crate::lv1::network`].
pub const NET_CONTROL: u64 = 194;

/// `connect_interrupt_event_receive_port`: four inputs and no output.
/// - r3 is the id of a bus.
/// - r4 is the id of a device on that bus.
/// - r5 is the id of an event receive port.
/// - r6 is a fourth input, which must be 0 for the network device.
///
/// The lv1 handler [OFW-4.93 lv1 a:0x305CF0] reads the inputs in this
/// form; see [`crate::lv1::network`].
pub const CONNECT_INTERRUPT_EVENT_RECEIVE_PORT: u64 = 197;
