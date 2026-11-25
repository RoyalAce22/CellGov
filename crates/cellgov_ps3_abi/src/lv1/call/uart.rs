//! The numbers of the virtual UART hypercalls.

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
