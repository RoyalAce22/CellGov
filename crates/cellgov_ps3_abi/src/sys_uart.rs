//! `sys_uart` (syscalls 367-370): the virtual UART to the system
//! controller's AV manager, and the PS3AV packet protocol spoken over
//! it -- command ids, status codes, event bits, and record layouts.
//!
//! Behaviour lives in `cellgov_lv2::host::uart`; this module is data
//! only.
//!
//! The PS3AV protocol is not a guest-facing API and nothing in the
//! corpus states it. The ring sizes, the header version and the
//! command ids below are all unestablished. The packets vsh sends over
//! this UART during boot would witness them.

/// Bytes the RX (reply) ring holds.
pub const PS3AV_RX_BUF_SIZE: usize = 0x800;

/// Bytes the TX (command) ring holds; a send larger than this cannot
/// be accepted whole.
pub const PS3AV_TX_BUF_SIZE: usize = 0x800;

/// Largest buffer either direction accepts in one syscall.
pub const SYS_UART_MAX_TRANSFER: u64 = 0x20000;

/// Bytes a big-op transfer moves per chunk; a non-blocking send whose
/// first chunk does not fit the TX ring reports that chunk's size.
pub const SYS_UART_CHUNK: u64 = 0x1000;

/// Protocol version every command header must carry.
pub const PS3AV_VERSION: u16 = 0x205;

/// `mode` for `sys_uart_receive` / `sys_uart_send`: transfer without
/// blocking, in 4 KiB chunks.
pub const SYS_UART_MODE_NOT_BLOCKING_BIG_OP: u32 = 0;
/// `mode`: block until the whole transfer completes.
pub const SYS_UART_MODE_BLOCKING_BIG_OP: u32 = 1;
/// `mode` (send only): accept the buffer whole or refuse it.
pub const SYS_UART_MODE_NOT_BLOCKING_OP: u32 = 2;

/// Command header: `version u16, length u16, cid u32`; `length` counts
/// the bytes after the first four.
pub const PS3AV_HEADER_LEN: usize = 8;
/// Reply header: the command header plus a `status u32`.
pub const PS3AV_REPLY_HEADER_LEN: usize = 12;
/// Set on a reply's cid.
pub const PS3AV_REPLY_BIT: u32 = 0x8000_0000;
/// `length` value the AV manager treats as a corrupt packet.
pub const PS3AV_LENGTH_POISON: u16 = 0xFFFC;
/// cid the corrupt-packet reply carries.
pub const PS3AV_CID_POISON_REPLY: u32 = 0xDEAD;

// Command ids.
/// PS3AV command id `AV_INIT`.
pub const PS3AV_CID_AV_INIT: u32 = 0x0000_0001;
/// PS3AV command id `AV_FIN`.
pub const PS3AV_CID_AV_FIN: u32 = 0x0000_0002;
/// PS3AV command id `AV_GET_HW_CONF`.
pub const PS3AV_CID_AV_GET_HW_CONF: u32 = 0x0000_0003;
/// PS3AV command id `AV_GET_MONITOR_INFO`.
pub const PS3AV_CID_AV_GET_MONITOR_INFO: u32 = 0x0000_0004;
/// PS3AV command id `AV_GET_BKSV_LIST`.
pub const PS3AV_CID_AV_GET_BKSV_LIST: u32 = 0x0000_0005;
/// PS3AV command id `AV_ENABLE_EVENT`.
pub const PS3AV_CID_AV_ENABLE_EVENT: u32 = 0x0000_0006;
/// PS3AV command id `AV_DISABLE_EVENT`.
pub const PS3AV_CID_AV_DISABLE_EVENT: u32 = 0x0000_0007;
/// PS3AV command id `AV_GET_PORT_STATE`.
pub const PS3AV_CID_AV_GET_PORT_STATE: u32 = 0x0000_0009;
/// PS3AV command id `AV_TV_MUTE`.
pub const PS3AV_CID_AV_TV_MUTE: u32 = 0x0000_000A;
/// PS3AV command id `AV_NULL_CMD`.
pub const PS3AV_CID_AV_NULL_CMD: u32 = 0x0000_000B;
/// PS3AV command id `AV_GET_AKSV`.
pub const PS3AV_CID_AV_GET_AKSV: u32 = 0x0000_000C;
/// PS3AV command id `AV_VIDEO_MUTE`.
pub const PS3AV_CID_AV_VIDEO_MUTE: u32 = 0x0001_0002;
/// PS3AV command id `AV_VIDEO_DISABLE_SIG`.
pub const PS3AV_CID_AV_VIDEO_DISABLE_SIG: u32 = 0x0001_0003;
/// PS3AV command id `AV_VIDEO_YTRAPCONTROL`.
pub const PS3AV_CID_AV_VIDEO_YTRAPCONTROL: u32 = 0x0001_0004;
/// PS3AV command id `AV_AUDIO_MUTE`.
pub const PS3AV_CID_AV_AUDIO_MUTE: u32 = 0x0002_0002;
/// PS3AV command id `AV_ACP_CTRL`.
pub const PS3AV_CID_AV_ACP_CTRL: u32 = 0x0002_0003;
/// PS3AV command id `AV_SET_ACP_PACKET`.
pub const PS3AV_CID_AV_SET_ACP_PACKET: u32 = 0x0002_0004;
/// PS3AV command id `AV_ADD_SIGNAL_CTL`.
pub const PS3AV_CID_AV_ADD_SIGNAL_CTL: u32 = 0x0003_0001;
/// PS3AV command id `AV_SET_CC_CODE`.
pub const PS3AV_CID_AV_SET_CC_CODE: u32 = 0x0003_0002;
/// PS3AV command id `AV_SET_CGMS_WSS`.
pub const PS3AV_CID_AV_SET_CGMS_WSS: u32 = 0x0003_0003;
/// PS3AV command id `AV_SET_MACROVISION`.
pub const PS3AV_CID_AV_SET_MACROVISION: u32 = 0x0003_0004;
/// PS3AV command id `AV_HDMI_MODE`.
pub const PS3AV_CID_AV_HDMI_MODE: u32 = 0x0004_0001;
/// PS3AV command id `AV_CEC_MESSAGE`.
pub const PS3AV_CID_AV_CEC_MESSAGE: u32 = 0x000A_0001;
/// PS3AV command id `AV_GET_CEC_CONFIG`.
pub const PS3AV_CID_AV_GET_CEC_CONFIG: u32 = 0x000A_0002;
/// PS3AV command id `AV_UNK11`.
pub const PS3AV_CID_AV_UNK11: u32 = 0x000A_0003;
/// PS3AV command id `AV_UNK12`.
pub const PS3AV_CID_AV_UNK12: u32 = 0x000A_0004;
/// PS3AV command id `VIDEO_INIT`.
pub const PS3AV_CID_VIDEO_INIT: u32 = 0x0100_0001;
/// PS3AV command id `VIDEO_MODE`.
pub const PS3AV_CID_VIDEO_MODE: u32 = 0x0100_0002;
/// PS3AV command id `VIDEO_ROUTE`.
pub const PS3AV_CID_VIDEO_ROUTE: u32 = 0x0100_0003;
/// PS3AV command id `VIDEO_FORMAT`.
pub const PS3AV_CID_VIDEO_FORMAT: u32 = 0x0100_0004;
/// PS3AV command id `VIDEO_PITCH`.
pub const PS3AV_CID_VIDEO_PITCH: u32 = 0x0100_0005;
/// PS3AV command id `VIDEO_GET_HW_CONF`.
pub const PS3AV_CID_VIDEO_GET_HW_CONF: u32 = 0x0100_0006;
/// PS3AV command id `VIDEO_GET_REG`.
pub const PS3AV_CID_VIDEO_GET_REG: u32 = 0x0100_0008;
/// PS3AV command id `AUDIO_INIT`.
pub const PS3AV_CID_AUDIO_INIT: u32 = 0x0200_0001;
/// PS3AV command id `AUDIO_MODE`.
pub const PS3AV_CID_AUDIO_MODE: u32 = 0x0200_0002;
/// PS3AV command id `AUDIO_MUTE`.
pub const PS3AV_CID_AUDIO_MUTE: u32 = 0x0200_0003;
/// PS3AV command id `AUDIO_ACTIVE`.
pub const PS3AV_CID_AUDIO_ACTIVE: u32 = 0x0200_0004;
/// PS3AV command id `AUDIO_INACTIVE`.
pub const PS3AV_CID_AUDIO_INACTIVE: u32 = 0x0200_0005;
/// PS3AV command id `AUDIO_SPDIF_BIT`.
pub const PS3AV_CID_AUDIO_SPDIF_BIT: u32 = 0x0200_0006;
/// PS3AV command id `AUDIO_CTRL`.
pub const PS3AV_CID_AUDIO_CTRL: u32 = 0x0200_0007;
/// PS3AV command id `AVB_PARAM`.
pub const PS3AV_CID_AVB_PARAM: u32 = 0x0400_0001;
/// Event the AV manager pushes into the stream: `UNPLUGGED`.
pub const PS3AV_CID_EVENT_UNPLUGGED: u32 = 0x1000_0001;
/// Event the AV manager pushes into the stream: `PLUGGED`.
pub const PS3AV_CID_EVENT_PLUGGED: u32 = 0x1000_0002;
/// Event the AV manager pushes into the stream: `HDCP_DONE`.
pub const PS3AV_CID_EVENT_HDCP_DONE: u32 = 0x1000_0003;
/// Event the AV manager pushes into the stream: `HDCP_FAIL`.
pub const PS3AV_CID_EVENT_HDCP_FAIL: u32 = 0x1000_0004;
/// Event the AV manager pushes into the stream: `HDCP_REAUTH`.
pub const PS3AV_CID_EVENT_HDCP_REAUTH: u32 = 0x1000_0005;
/// Event the AV manager pushes into the stream: `HDCP_ERROR`.
pub const PS3AV_CID_EVENT_HDCP_ERROR: u32 = 0x1000_0006;
/// OR-ed into an event cid when it concerns the second HDMI port.
pub const PS3AV_CID_EVENT_HDMI_1_BIT: u32 = 0x0001_0000;

// Reply status codes.
/// Reply status `SUCCESS`.
pub const PS3AV_STATUS_SUCCESS: u32 = 0x00;
/// Reply status `RECEIVE_VUART_ERROR`.
pub const PS3AV_STATUS_RECEIVE_VUART_ERROR: u32 = 0x01;
/// Reply status `SYSCON_COMMUNICATE_FAIL`.
pub const PS3AV_STATUS_SYSCON_COMMUNICATE_FAIL: u32 = 0x02;
/// Reply status `INVALID_COMMAND`.
pub const PS3AV_STATUS_INVALID_COMMAND: u32 = 0x03;
/// Reply status `INVALID_PORT`.
pub const PS3AV_STATUS_INVALID_PORT: u32 = 0x04;
/// Reply status `INVALID_VID`.
pub const PS3AV_STATUS_INVALID_VID: u32 = 0x05;
/// Reply status `INVALID_COLOR_SPACE`.
pub const PS3AV_STATUS_INVALID_COLOR_SPACE: u32 = 0x06;
/// Reply status `INVALID_FS`.
pub const PS3AV_STATUS_INVALID_FS: u32 = 0x07;
/// Reply status `INVALID_AUDIO_CH`.
pub const PS3AV_STATUS_INVALID_AUDIO_CH: u32 = 0x08;
/// Reply status `UNSUPPORTED_VERSION`.
pub const PS3AV_STATUS_UNSUPPORTED_VERSION: u32 = 0x09;
/// Reply status `INVALID_SAMPLE_SIZE`.
pub const PS3AV_STATUS_INVALID_SAMPLE_SIZE: u32 = 0x0A;
/// Reply status `FAILURE`.
pub const PS3AV_STATUS_FAILURE: u32 = 0x0B;
/// Reply status `UNSUPPORTED_COMMAND`.
pub const PS3AV_STATUS_UNSUPPORTED_COMMAND: u32 = 0x0C;
/// Reply status `BUFFER_OVERFLOW`.
pub const PS3AV_STATUS_BUFFER_OVERFLOW: u32 = 0x0D;
/// Reply status `INVALID_VIDEO_PARAM`.
pub const PS3AV_STATUS_INVALID_VIDEO_PARAM: u32 = 0x0E;
/// Reply status `NO_SEL`.
pub const PS3AV_STATUS_NO_SEL: u32 = 0x0F;
/// Reply status `INVALID_AV_PARAM`.
pub const PS3AV_STATUS_INVALID_AV_PARAM: u32 = 0x10;
/// Reply status `INVALID_AUDIO_PARAM`.
pub const PS3AV_STATUS_INVALID_AUDIO_PARAM: u32 = 0x11;
/// Reply status `UNSUPPORTED_HDMI_MODE`.
pub const PS3AV_STATUS_UNSUPPORTED_HDMI_MODE: u32 = 0x12;
/// Reply status `NO_SYNC_HEAD`.
pub const PS3AV_STATUS_NO_SYNC_HEAD: u32 = 0x13;

// AV ports as the packets name them.
/// AV port `HDMI_0` as the packets name it.
pub const PS3AV_AVPORT_HDMI_0: u16 = 0x00;
/// AV port `HDMI_1` as the packets name it.
pub const PS3AV_AVPORT_HDMI_1: u16 = 0x01;
/// AV port `AVMULTI_0` as the packets name it.
pub const PS3AV_AVPORT_AVMULTI_0: u16 = 0x10;
/// AV port `AVMULTI_1` as the packets name it.
pub const PS3AV_AVPORT_AVMULTI_1: u16 = 0x11;
/// AV port `SPDIF_0` as the packets name it.
pub const PS3AV_AVPORT_SPDIF_0: u16 = 0x20;
/// AV port `SPDIF_1` as the packets name it.
pub const PS3AV_AVPORT_SPDIF_1: u16 = 0x21;

// Video heads.
/// Video head `A_HDMI`.
pub const PS3AV_HEAD_A_HDMI: u32 = 0;
/// Video head `B_ANALOG`.
pub const PS3AV_HEAD_B_ANALOG: u32 = 1;

// Event-enable bits carried by AV_INIT / ENABLE_EVENT / DISABLE_EVENT.
/// Enabled-event bit `UNPLUGGED`.
pub const PS3AV_EVENT_BIT_UNPLUGGED: u32 = 0x01;
/// Enabled-event bit `PLUGGED`.
pub const PS3AV_EVENT_BIT_PLUGGED: u32 = 0x02;
/// Enabled-event bit `HDCP_DONE`.
pub const PS3AV_EVENT_BIT_HDCP_DONE: u32 = 0x04;
/// Enabled-event bit `HDCP_FAIL`.
pub const PS3AV_EVENT_BIT_HDCP_FAIL: u32 = 0x08;
/// Enabled-event bit `HDCP_REAUTH`.
pub const PS3AV_EVENT_BIT_HDCP_REAUTH: u32 = 0x10;
/// Enabled-event bit `HDCP_TOPOLOGY`.
pub const PS3AV_EVENT_BIT_HDCP_TOPOLOGY: u32 = 0x20;
/// AV_INIT with this bit expects a 4-byte reply body.
pub const PS3AV_EVENT_BIT_UNK: u32 = 0x8000_0000;

// HDMI behaviour modes (AV_HDMI_MODE).
/// HDMI behaviour mode `HDCP_OFF`.
pub const PS3AV_HDMI_BEHAVIOR_HDCP_OFF: u8 = 0x01;
/// HDMI behaviour mode `DVI`.
pub const PS3AV_HDMI_BEHAVIOR_DVI: u8 = 0x40;
/// HDMI behaviour mode `EDID_PASS`.
pub const PS3AV_HDMI_BEHAVIOR_EDID_PASS: u8 = 0x80;
/// HDMI behaviour mode `NORMAL`.
pub const PS3AV_HDMI_BEHAVIOR_NORMAL: u8 = 0xFF;

// Monitor descriptor vocabulary.
/// Monitor type `NONE`.
pub const PS3AV_MONITOR_TYPE_NONE: u8 = 0;
/// Monitor type `HDMI`.
pub const PS3AV_MONITOR_TYPE_HDMI: u8 = 1;
/// Monitor type `DVI`.
pub const PS3AV_MONITOR_TYPE_DVI: u8 = 2;
/// Monitor type `AVMULTI`.
pub const PS3AV_MONITOR_TYPE_AVMULTI: u8 = 3;
/// Resolution bit `720X480P` in a monitor descriptor.
pub const PS3AV_RESBIT_720X480P: u32 = 0x0003;
/// Resolution bit `720X576P` in a monitor descriptor.
pub const PS3AV_RESBIT_720X576P: u32 = 0x0003;
/// Resolution bit `1280X720P` in a monitor descriptor.
pub const PS3AV_RESBIT_1280X720P: u32 = 0x0004;
/// Resolution bit `1920X1080I` in a monitor descriptor.
pub const PS3AV_RESBIT_1920X1080I: u32 = 0x0008;
/// Resolution bit `1920X1080P` in a monitor descriptor.
pub const PS3AV_RESBIT_1920X1080P: u32 = 0x4000;
/// Colorimetry flag `XVYCC_601` in a monitor descriptor.
pub const PS3AV_COLORIMETRY_XVYCC_601: u8 = 1;
/// Colorimetry flag `XVYCC_709` in a monitor descriptor.
pub const PS3AV_COLORIMETRY_XVYCC_709: u8 = 2;
/// Colorimetry flag `MD0` in a monitor descriptor.
pub const PS3AV_COLORIMETRY_MD0: u8 = 1 << 4;
/// Colorimetry flag `MD1` in a monitor descriptor.
pub const PS3AV_COLORIMETRY_MD1: u8 = 1 << 5;
/// Colorimetry flag `MD2` in a monitor descriptor.
pub const PS3AV_COLORIMETRY_MD2: u8 = 1 << 6;
/// Monitor descriptor flag `PS3AV_CS_SUPPORTED`.
pub const PS3AV_CS_SUPPORTED: u8 = 1;
/// Monitor descriptor flag `PS3AV_RGB_SELECTABLE_QUANTIZATION_RANGE`.
pub const PS3AV_RGB_SELECTABLE_QUANTIZATION_RANGE: u8 = 8;
/// Monitor descriptor flag `PS3AV_12BIT_COLOR`.
pub const PS3AV_12BIT_COLOR: u8 = 16;
/// Monitor descriptor bound `AUDIO_BLK_MAX`.
pub const PS3AV_MON_INFO_AUDIO_BLK_MAX: usize = 16;
/// Audio block type `LPCM` in a monitor descriptor.
pub const PS3AV_MON_INFO_AUDIO_TYPE_LPCM: u8 = 1;
/// Audio block type `AC3` in a monitor descriptor.
pub const PS3AV_MON_INFO_AUDIO_TYPE_AC3: u8 = 2;
/// Audio block type `AAC` in a monitor descriptor.
pub const PS3AV_MON_INFO_AUDIO_TYPE_AAC: u8 = 6;
/// Audio block type `DTS` in a monitor descriptor.
pub const PS3AV_MON_INFO_AUDIO_TYPE_DTS: u8 = 7;
/// Audio block type `DDP` in a monitor descriptor.
pub const PS3AV_MON_INFO_AUDIO_TYPE_DDP: u8 = 10;
/// Audio block type `DTS_HD` in a monitor descriptor.
pub const PS3AV_MON_INFO_AUDIO_TYPE_DTS_HD: u8 = 11;
/// Audio block type `DOLBY_THD` in a monitor descriptor.
pub const PS3AV_MON_INFO_AUDIO_TYPE_DOLBY_THD: u8 = 12;

/// `ps3av_get_monitor_info_reply` is 208 bytes; the HDMI reply and the
/// plugged event carry it four bytes short.
pub const PS3AV_MONITOR_INFO_LEN: usize = 208;
/// Bytes of monitor info the HDMI reply and the plugged event carry.
pub const PS3AV_MONITOR_INFO_HDMI_LEN: usize = 204;

/// KSV of the HDMI transmitter, as GET_AKSV reports it.
pub const PS3AV_AKSV_VALUE: [u8; 5] = [0x00, 0x00, 0x0F, 0xFF, 0xFF];
/// KSV of the single HDCP sink, as BKSV lists and HDCP events report it.
pub const PS3AV_BKSV_VALUE: [u8; 5] = [0xFF, 0xFF, 0xF0, 0x00, 0x00];

// Audio-mode vocabulary (AUDIO_MODE packet).
/// Audio-mode value `FS_192K`.
pub const PS3AV_AUDIO_FS_192K: u32 = 7;
/// Audio-mode value `SOURCE_SERIAL`.
pub const PS3AV_AUDIO_SOURCE_SERIAL: u32 = 0;
/// Audio-mode value `SOURCE_SPDIF`.
pub const PS3AV_AUDIO_SOURCE_SPDIF: u32 = 1;
/// Audio-port bits for AUDIO_ACTIVE / AUDIO_INACTIVE / AUDIO_SPDIF_BIT.
pub const PS3AV_AUDIO_PORT_HDMI_0: u32 = 1 << 0;
/// Audio-port bit `HDMI_1`.
pub const PS3AV_AUDIO_PORT_HDMI_1: u32 = 1 << 1;
/// Audio-port bit `AVMULTI`.
pub const PS3AV_AUDIO_PORT_AVMULTI: u32 = 1 << 10;
/// Audio-port bit `SPDIF_0`.
pub const PS3AV_AUDIO_PORT_SPDIF_0: u32 = 1 << 20;
/// Audio-port bit `SPDIF_1`.
pub const PS3AV_AUDIO_PORT_SPDIF_1: u32 = 1 << 21;

// Fixed packet sizes the parser checks a command against (header
// included); a command whose size is data-dependent or unchecked is
// absent here.
/// Size of the `AV_INIT` command packet, header included.
pub const PS3AV_PKT_AV_INIT_LEN: usize = 12;
/// Size of the `GET_MONITOR_INFO` command packet, header included.
pub const PS3AV_PKT_GET_MONITOR_INFO_LEN: usize = 12;
/// Size of the `GET_BKSV` command packet, header included.
pub const PS3AV_PKT_GET_BKSV_LEN: usize = 12;
/// Size of the `ENABLE_EVENT` command packet, header included.
pub const PS3AV_PKT_ENABLE_EVENT_LEN: usize = 12;
/// Size of the `AV_AUDIO_MUTE` command packet, header included.
pub const PS3AV_PKT_AV_AUDIO_MUTE_LEN: usize = 12;
/// Size of the `NULL_CMD` command packet, header included.
pub const PS3AV_PKT_NULL_CMD_LEN: usize = 12;
/// Size of the `VIDEO_DISABLE_SIG` command packet, header included.
pub const PS3AV_PKT_VIDEO_DISABLE_SIG_LEN: usize = 12;
/// Size of the `YTRAPCONTROL` command packet, header included.
pub const PS3AV_PKT_YTRAPCONTROL_LEN: usize = 12;
/// Size of the `ACP_CTRL` command packet, header included.
pub const PS3AV_PKT_ACP_CTRL_LEN: usize = 12;
/// Size of the `SET_ACP_PACKET` command packet, header included.
pub const PS3AV_PKT_SET_ACP_PACKET_LEN: usize = 44;
/// Size of the `ADD_SIGNAL_CTL` command packet, header included.
pub const PS3AV_PKT_ADD_SIGNAL_CTL_LEN: usize = 12;
/// Size of the `SET_CGMS_WSS` command packet, header included.
pub const PS3AV_PKT_SET_CGMS_WSS_LEN: usize = 16;
/// Size of the `SET_HDMI_MODE` command packet, header included.
pub const PS3AV_PKT_SET_HDMI_MODE_LEN: usize = 12;
/// Size of the `VIDEO_FORMAT` command packet, header included.
pub const PS3AV_PKT_VIDEO_FORMAT_LEN: usize = 20;
/// Size of the `VIDEO_ROUTE` command packet, header included.
pub const PS3AV_PKT_VIDEO_ROUTE_LEN: usize = 24;
/// Size of the `VIDEO_PITCH` command packet, header included.
pub const PS3AV_PKT_VIDEO_PITCH_LEN: usize = 16;
/// Size of the `AUDIO_MODE` command packet, header included.
pub const PS3AV_PKT_AUDIO_MODE_LEN: usize = 68;
/// Size of the `AUDIO_SET_ACTIVE` command packet, header included.
pub const PS3AV_PKT_AUDIO_SET_ACTIVE_LEN: usize = 12;
/// Size of the `AUDIO_SPDIF_BIT` command packet, header included.
pub const PS3AV_PKT_AUDIO_SPDIF_BIT_LEN: usize = 64;
/// Size of the `AUDIO_CTRL` command packet, header included.
pub const PS3AV_PKT_AUDIO_CTRL_LEN: usize = 28;
/// Size of the `INC_AVSET` command packet, header included.
pub const PS3AV_PKT_INC_AVSET_LEN: usize = 16;
/// Size of the `VIDEO_MODE` command packet, header included.
pub const PS3AV_PKT_VIDEO_MODE_LEN: usize = 48;
/// Size of the `AV_VIDEO_CS` command packet, header included.
pub const PS3AV_PKT_AV_VIDEO_CS_LEN: usize = 24;
/// Size of the `AV_AUDIO_PARAM` command packet, header included.
pub const PS3AV_PKT_AV_AUDIO_PARAM_LEN: usize = 32;

// Reply body sizes.
/// Length of the `AV_INIT` reply body.
pub const PS3AV_REPLY_AV_INIT_LEN: usize = 4;
/// Length of the `GET_HW_CONF` reply body.
pub const PS3AV_REPLY_GET_HW_CONF_LEN: usize = 8;
/// Length of the `GET_AKSV` reply body.
pub const PS3AV_REPLY_GET_AKSV_LEN: usize = 16;
/// Length of the `VIDEO_GET_HW_CONF` reply body.
pub const PS3AV_REPLY_VIDEO_GET_HW_CONF_LEN: usize = 4;
/// Length of the `GET_CEC_CONFIG` reply body.
pub const PS3AV_REPLY_GET_CEC_CONFIG_LEN: usize = 4;
/// `ps3av_pkt_get_bksv_reply` ahead of its KSV array.
pub const PS3AV_REPLY_GET_BKSV_HEAD_LEN: usize = 8;
/// `ps3av_pkt_hdmi_hdcp_done_event` ahead of its KSV array.
pub const PS3AV_EVENT_HDCP_DONE_HEAD_LEN: usize = 12;

/// `sys_uart_get_params` output: `rx_buf_size u64, tx_buf_size u64`.
pub const SYS_UART_PARAMS_LEN: usize = 16;
