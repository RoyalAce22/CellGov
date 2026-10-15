//! `sys_uart` (367-370): the virtual UART to the system controller's
//! AV manager, spoken in PS3AV packets.
//!
//! `sys_uart_send` parses every packet in the buffer at dispatch and
//! stages the replies; `sys_uart_receive` hands the reply stream back,
//! parking a blocking caller until a send stages bytes; several may
//! park, and bytes go to them in park order. HDMI plug and
//! HDCP events follow their triggering command's replies in the
//! stream and are gated by the enabled-event mask at that moment. The
//! monitor on HDMI 0 and the AV-multi port are fixed fixtures; audio
//! and video packets are validated and acknowledged but drive nothing.
//!
//! On a console the far end of this UART is the system controller,
//! which answers on its own schedule. Its firmware is not part of
//! dev_flash, so nothing here has a witness: the host stages every
//! reply at dispatch, in one order, with no latency.
//!
//! No probe sends `sys_uart_initialize`, `sys_uart_receive` or
//! `sys_uart_send`: they reach the system controller and the AV manager
//! behind it, so the runner refuses a probe that names them. Only a
//! passive capture of what vsh sends may settle their answers, the
//! reply contents and the parked-reader order. `sys_uart_get_params`
//! reads two sizes, and the `lv2_uart_get_params` capture settles its
//! answer to a game process after vsh has claimed the UART.
//!
//! vsh.self is the only dev_flash module that speaks this UART. It
//! retries `sys_uart_initialize` up to ten times, with a 100 ms sleep
//! between attempts. It has one send wrapper (mode 2) and one receive
//! wrapper (mode 1), so no other mode has a firmware caller.

mod av_data;
mod cid_handlers;
mod cid_table;
mod hdmi;
mod packet;
mod reply;
mod state;
mod syscalls;

pub(crate) use state::UartState;

#[cfg(test)]
#[path = "tests/uart_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/uart_monitor_layout_tests.rs"]
mod monitor_layout_tests;

#[cfg(test)]
#[path = "tests/uart_reader_queue_tests.rs"]
mod reader_queue_tests;

#[cfg(test)]
#[path = "tests/uart_purge_tests.rs"]
mod purge_tests;
