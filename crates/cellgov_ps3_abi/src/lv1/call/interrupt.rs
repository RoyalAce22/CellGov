//! The numbers of the interrupt delivery hypercalls.

/// `configure_irq_state_bitmap`: register the interrupt state bitmap of
/// one hardware thread of a logical PPE.
///
/// The inputs:
/// - r3 is the logical PPE id.
/// - r4 is the hardware thread.
/// - r5 is the bitmap's logical partition address.
///
/// There is no output. Linux `arch/powerpc/platforms/ps3/interrupt.c`,
/// `ps3_init_IRQ`, makes the call in this form. The lv1 handler
/// [OFW-4.93 lv1 a:0x2EA35C] reads the inputs in this form; see
/// [`crate::lv1::interrupt`].
pub const CONFIGURE_IRQ_STATE_BITMAP: u64 = 11;

/// `connect_irq_plug_ext`: five inputs and no output.
/// - r3 is the logical PPE id.
/// - r4 is the hardware thread.
/// - r5 is the plug.
/// - r6 is the id of the event receive port.
/// - r7 is 0.
///
/// Linux `arch/powerpc/platforms/ps3/interrupt.c`, `ps3_irq_plug_setup`,
/// makes the call in this form. The lv1 handler [OFW-4.93 lv1 a:0x2E9DA8]
/// reads the inputs in this form; see [`crate::lv1::irq_plug`].
pub const CONNECT_IRQ_PLUG_EXT: u64 = 12;

/// `construct_event_receive_port`: no input, and the new port's id in
/// r4. Linux `arch/powerpc/platforms/ps3/interrupt.c`,
/// `ps3_event_receive_port_setup`, makes the call in this form and names
/// the output the outlet. The lv1 handler [OFW-4.93 lv1 a:0x2EF424]
/// writes the output in this form; see [`crate::lv1::event_port`].
pub const CONSTRUCT_EVENT_RECEIVE_PORT: u64 = 18;

/// `did_update_interrupt_mask`: two inputs and no output.
/// - r3 is the logical PPE id.
/// - r4 is the hardware thread whose interrupt mask changed.
///
/// Linux `arch/powerpc/platforms/ps3/interrupt.c`, `ps3_chip_mask`, makes
/// the call in this form after it changes the mask of its bitmap. The lv1
/// handler [OFW-4.93 lv1 a:0x2E9354] reads the inputs in this form; see
/// [`crate::lv1::interrupt_mask`].
pub const DID_UPDATE_INTERRUPT_MASK: u64 = 31;
