//! `sys_config` (syscalls 516-522): service ids, listener types,
//! event sources, and the service-event record layout.
//!
//! Behaviour (the subscription store) lives in
//! `cellgov_lv2::host::config`; this module is data only.
//!
//! libio.sprx, sys_audio.self and vsh.self all call sys_config, so the
//! syscalls have firmware callers. Those callers are clients: nothing
//! in dev_flash states the record layout or the buffer floor below,
//! which are the kernel's side.

/// LV2-provided pad-manager service; the listener's data buffer must
/// lead with `0x01` to receive its events.
pub const SYS_CONFIG_SERVICE_PADMANAGER: u64 = 0x11;

/// Second pad-manager service id; LV2 mirrors pad events to both.
pub const SYS_CONFIG_SERVICE_PADMANAGER2: u64 = 0x12;

/// Top bit set: a user-registered service rather than an LV2 one.
pub const SYS_CONFIG_SERVICE_USER_BASE: u64 = 0x8000_0000_0000_0000;

/// libpad's user service id.
pub const SYS_CONFIG_SERVICE_USER_LIBPAD: u64 = SYS_CONFIG_SERVICE_USER_BASE + 1;

/// `type = SYS_CONFIG_SERVICE_LISTENER_ONCE`: the listener receives
/// at most one event.
pub const SYS_CONFIG_SERVICE_LISTENER_ONCE: u32 = 0;

/// `type = SYS_CONFIG_SERVICE_LISTENER_REPEATING`: every matching
/// registration and unregistration is delivered.
pub const SYS_CONFIG_SERVICE_LISTENER_REPEATING: u32 = 1;

/// `sys_event_t.source` for a service registration event.
pub const SYS_CONFIG_EVENT_SOURCE_SERVICE: u64 = 1;

/// `sys_event_t.source` for an IO event (523-525, unmodeled).
pub const SYS_CONFIG_EVENT_SOURCE_IO: u64 = 2;

/// Bytes of `sys_config_service_event_t` ahead of its `data` array:
/// listener handle u32, registered u32, service id u64, user id u64,
/// verbosity u64, data size u32, padding u32.
pub const SYS_CONFIG_SERVICE_EVENT_HEAD_LEN: usize = 40;

/// Bytes written for an unregistered service's event: the record
/// ends after `user_id`.
pub const SYS_CONFIG_SERVICE_EVENT_UNREGISTERED_LEN: usize = 24;

/// `sizeof(sys_config_service_event_t)`: the 40-byte head, one byte of
/// `data`, and seven bytes of tail padding from 8-byte alignment.
///
/// The queued event's `data3` announces `SIZEOF - 1 + data.len()`, and
/// `sys_config_get_service_event` refuses anything smaller with
/// `CELL_EAGAIN`. The demanded length therefore runs seven bytes past
/// the bytes the kernel writes. No firmware caller witnesses that gap:
/// libio's sc 518 call site never sizes a buffer to the bytes written.
pub const SYS_CONFIG_SERVICE_EVENT_SIZEOF: usize = 48;

/// Bytes the queued event's `data3` announces ahead of the service
/// data, and the `sys_config_get_service_event` buffer floor; see
/// [`SYS_CONFIG_SERVICE_EVENT_SIZEOF`].
pub const SYS_CONFIG_SERVICE_EVENT_ANNOUNCED_HEAD_LEN: usize = SYS_CONFIG_SERVICE_EVENT_SIZEOF - 1;

/// Synthetic pad-manager descriptor registered on both services for
/// port 0 at first `sys_config_open`, so the shell sees one connected
/// controller: a DUALSHOCK 3, vid `0x054c` pid `0x0268`.
///
/// CellGov invents these bytes. A console reads them from the attached
/// device, so no descriptor in dev_flash carries this vid/pid pair.
/// CellGov acts on no byte of the descriptor: event delivery is gated
/// on the listener's own data buffer, not on this one.
pub const SYS_CONFIG_PADMANAGER_DS3_DESCRIPTOR: [u8; 26] = [
    0x01, 0x01, 0x02, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x4c, 0x02, 0x68, 0x00, 0x10,
    0x91, 0x88, 0x04, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00,
];
