//! The numbers of the virtual UART hypercalls.

/// `read_virtual_uart`: three inputs and one output.
/// - r3 is the number of a port of the partition's virtual UART.
/// - r4 is the logical partition address of the buffer that receives the
///   bytes.
/// - r5 is the largest number of bytes to read.
/// - r4 returns the number of bytes that lv1 read.
///
/// Linux `drivers/ps3/ps3-vuart.c` makes the call in this form. The lv1
/// handler [OFW-4.93 lv1 a:0x2F429C] reads the inputs in this form; see
/// [`crate::lv1::uart`].
pub const READ_VIRTUAL_UART: u64 = 162;

/// `write_virtual_uart`: three inputs and one output.
/// - r3 is the number of a port of the partition's virtual UART.
/// - r4 is the logical partition address of the bytes to send.
/// - r5 is the number of bytes to send.
/// - r4 returns the number of bytes that lv1 wrote.
///
/// Linux `drivers/ps3/ps3-vuart.c` makes the call in this form. The lv1
/// handler [OFW-4.93 lv1 a:0x2F2AF4] reads the inputs in this form; see
/// [`crate::lv1::uart`].
pub const WRITE_VIRTUAL_UART: u64 = 163;

/// `set_virtual_uart_param`: three inputs and no output.
/// - r3 is the number of a port of the partition's virtual UART.
/// - r4 is the id of the port's parameter.
/// - r5 is the parameter's new value.
///
/// Linux `drivers/ps3/ps3-vuart.c` makes the call in this form. The lv1
/// handler [OFW-4.93 lv1 a:0x2F4100] reads the inputs in this form; see
/// [`crate::lv1::uart`].
pub const SET_VIRTUAL_UART_PARAM: u64 = 164;

/// `get_virtual_uart_param`: two inputs and one output.
/// - r3 is the number of a port of the partition's virtual UART.
/// - r4 is the id of the port's parameter.
/// - r4 returns the parameter's value.
///
/// Linux `drivers/ps3/ps3-vuart.c` makes the call in this form, and its
/// `enum vuart_param` names the ids. The lv1 handler
/// [OFW-4.93 lv1 a:0x2F3A68] reads the inputs in this form; see
/// [`crate::lv1::uart`].
pub const GET_VIRTUAL_UART_PARAM: u64 = 165;

/// `configure_virtual_uart_irq`: one input and one output.
/// - r3 is the logical partition address of the partition's 32-byte
///   virtual UART interrupt bitmap.
/// - r4 returns the id of the irq outlet that lv1 constructs for the
///   virtual UART.
///
/// Linux `arch/powerpc/platforms/ps3/interrupt.c`, `ps3_vuart_irq_setup`,
/// makes the call in this form. The lv1 handler [OFW-4.93 lv1 a:0x2F2810]
/// reads the input in this form; see [`crate::lv1::uart`].
pub const CONFIGURE_VIRTUAL_UART_IRQ: u64 = 166;
