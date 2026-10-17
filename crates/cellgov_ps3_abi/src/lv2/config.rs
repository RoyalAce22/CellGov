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

/// LV2-provided pad-manager service. A listener with no data buffer
/// receives its events, as the `lv2_config_listener_shapes` capture
/// shows.
pub const SYS_CONFIG_SERVICE_PADMANAGER: u64 = 0x11;

/// Second pad-manager service id. On the reference console a listener
/// on it with no data buffer receives one event for the attached pad,
/// as one on [`SYS_CONFIG_SERVICE_PADMANAGER`] does
/// (`lv2_config_service_records`).
pub const SYS_CONFIG_SERVICE_PADMANAGER2: u64 = 0x12;

/// Top bit set: a user-registered service rather than an LV2 one. On
/// the reference console a game registers both a listener and a service
/// at `USER_BASE + 0x4347` (`lv2_config_service_records`).
pub const SYS_CONFIG_SERVICE_USER_BASE: u64 = 0x8000_0000_0000_0000;

/// libpad's user service id. A game's listener on it registers and,
/// with no libpad running, receives nothing (`lv2_config_service_records`).
pub const SYS_CONFIG_SERVICE_USER_LIBPAD: u64 = SYS_CONFIG_SERVICE_USER_BASE + 1;

/// `type = SYS_CONFIG_SERVICE_LISTENER_ONCE`: the listener receives
/// at most one event. On the reference console a once listener on the
/// pad manager receives the pad's event (`lv2_config_service_records`).
pub const SYS_CONFIG_SERVICE_LISTENER_ONCE: u32 = 0;

/// `type = SYS_CONFIG_SERVICE_LISTENER_REPEATING`: every matching
/// registration and unregistration is delivered. The reference console
/// registers a listener of any other type too, type 2 among them, and
/// delivers it nothing (`lv2_config_service_records`).
pub const SYS_CONFIG_SERVICE_LISTENER_REPEATING: u32 = 1;

/// `sys_event_t.source` for a service registration event, as the
/// `lv2_config_listener_shapes` and `lv2_config_service_records`
/// captures read it.
pub const SYS_CONFIG_EVENT_SOURCE_SERVICE: u64 = 1;

/// `sys_event_t.source` for an IO event (523-525, unmodeled). No
/// capture has one: an IO event needs a device attached or removed
/// while the probe runs.
pub const SYS_CONFIG_EVENT_SOURCE_IO: u64 = 2;

/// Bytes of `sys_config_service_event_t` ahead of its `data` array:
/// listener handle u32, registered u32, service id u64, user id u64,
/// verbosity u64, data size u32, padding u32.
///
/// The queued event's `data3` announces this head plus the data
/// length, which is also the length written, as the
/// `lv2_config_listener_shapes` capture shows.
pub const SYS_CONFIG_SERVICE_EVENT_HEAD_LEN: usize = 40;

/// Bytes written for an unregistered service's event: the record
/// ends after `user_id`. No capture shows one. A game's own service
/// unregistering delivers no event to its own listener
/// (`lv2_config_service_records`), and the pad manager's unregisters
/// only when the controller is removed.
pub const SYS_CONFIG_SERVICE_EVENT_UNREGISTERED_LEN: usize = 24;

/// Pad-manager descriptor registered on both services for port 0 at
/// first `sys_config_open`, so the shell sees one connected controller:
/// a DUALSHOCK 3, vid `0x054c` pid `0x0268`.
///
/// Bytes 0 to 14 are the ones the reference console announced for its
/// attached DUALSHOCK 3 (`lv2_config_service_records`). The bytes from
/// 15 on differ from one controller to the next and may identify the
/// one attached, so the probe does not record them; CellGov's are its
/// own pick. CellGov acts on no byte of the descriptor: event delivery
/// is gated on the listener's own data buffer, not on this one.
pub const SYS_CONFIG_PADMANAGER_DS3_DESCRIPTOR: [u8; 26] = [
    0x01, 0x01, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x4c, 0x02, 0x68, 0x00, 0x10,
    0x91, 0x88, 0x04, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00,
];

/// Bytes of [`SYS_CONFIG_PADMANAGER_DS3_DESCRIPTOR`] a console capture
/// settles; the rest differ per controller.
pub const SYS_CONFIG_PADMANAGER_DS3_DESCRIPTOR_CAPTURED: usize = 15;
