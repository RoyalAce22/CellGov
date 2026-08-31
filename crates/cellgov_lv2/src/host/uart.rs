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

use std::collections::VecDeque;

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors;
use cellgov_ps3_abi::sys_uart as av;
use cellgov_time::GuestTicks;

use crate::dispatch::{Lv2BlockReason, Lv2Dispatch, PendingResponse};
use crate::host::{Lv2Host, Lv2Runtime};
use crate::ppu_thread::PpuThreadId;

/// HDMI link states the event script walks, in order; 0 is the
/// "before any state" floor the script can start from.
const HDMI_STATE_UNPLUGGED: u8 = 1;
const HDMI_STATE_PLUGGED: u8 = 2;
const HDMI_STATE_HDCP_DONE: u8 = 3;

/// A blocking reader parked on an empty reply stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UartReader {
    pub thread: PpuThreadId,
    pub buf_ptr: u32,
    pub size: u64,
}

/// Reply stream, parked readers, and the AV manager's HDMI state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UartState {
    initialized: bool,
    /// Undelivered reply and event bytes, oldest first.
    rx: Vec<u8>,
    /// Blocking readers in park order; the front takes the next
    /// bytes. Which of several parked readers wins a send is a race on
    /// a console, so park order here is CellGov's own.
    readers: VecDeque<UartReader>,
    /// Version the last AV_INIT carried; events echo it.
    av_cmd_ver: u16,
    /// Enabled-event mask (`PS3AV_EVENT_BIT_*`).
    hdmi_events: u32,
    hdmi_behavior: u8,
    head_b_initialized: bool,
    hdmi_res_set: [bool; 2],
    hdcp_first_auth: [bool; 2],
    /// Last state the HDMI 0 event script was driven to; the next
    /// script starts one step below its own first state or here,
    /// whichever is lower.
    hdmi_to_state: u8,
}

impl UartState {
    pub(crate) fn new() -> Self {
        Self {
            initialized: false,
            rx: Vec::new(),
            readers: VecDeque::new(),
            av_cmd_ver: 0,
            hdmi_events: 0,
            hdmi_behavior: av::PS3AV_HDMI_BEHAVIOR_NORMAL,
            head_b_initialized: false,
            hdmi_res_set: [false; 2],
            hdcp_first_auth: [true; 2],
            hdmi_to_state: HDMI_STATE_PLUGGED,
        }
    }

    /// True until `sys_uart_initialize`; the state hash skips a
    /// pristine UART.
    pub(crate) fn is_pristine(&self) -> bool {
        *self == Self::new()
    }

    #[cfg(test)]
    pub(crate) fn pending_bytes(&self) -> &[u8] {
        &self.rx
    }

    #[cfg(test)]
    pub(crate) fn readers(&self) -> &VecDeque<UartReader> {
        &self.readers
    }

    /// Remove every parked reader whose thread is in `threads`,
    /// preserving the order of survivors; returns the removed
    /// records. Process-exit purge: a reader of an exited process
    /// would otherwise be served first and its bytes dropped with
    /// the wake, ahead of a live reader behind it.
    #[must_use = "the purged readers are the only witness that these wakes were cancelled"]
    pub(crate) fn purge_readers_of(
        &mut self,
        threads: &std::collections::BTreeSet<PpuThreadId>,
    ) -> Vec<UartReader> {
        let mut removed = Vec::new();
        self.readers.retain(|r| {
            if threads.contains(&r.thread) {
                removed.push(*r);
                false
            } else {
                true
            }
        });
        removed
    }

    #[cfg(test)]
    pub(crate) fn hdmi_events(&self) -> u32 {
        self.hdmi_events
    }

    /// FNV-1a over every field via raw little-endian bytes per the
    /// host state-hash contract.
    pub(crate) fn state_hash(&self) -> u64 {
        let Self {
            initialized,
            rx,
            readers,
            av_cmd_ver,
            hdmi_events,
            hdmi_behavior,
            head_b_initialized,
            hdmi_res_set,
            hdcp_first_auth,
            hdmi_to_state,
        } = self;
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&[u8::from(*initialized)]);
        hasher.write(&(rx.len() as u64).to_le_bytes());
        hasher.write(rx);
        hasher.write(&(readers.len() as u64).to_le_bytes());
        for r in readers {
            hasher.write(&r.thread.raw().to_le_bytes());
            hasher.write(&r.buf_ptr.to_le_bytes());
            hasher.write(&r.size.to_le_bytes());
        }
        hasher.write(&av_cmd_ver.to_le_bytes());
        hasher.write(&hdmi_events.to_le_bytes());
        hasher.write(&[*hdmi_behavior, u8::from(*head_b_initialized)]);
        hasher.write(&[u8::from(hdmi_res_set[0]), u8::from(hdmi_res_set[1])]);
        hasher.write(&[u8::from(hdcp_first_auth[0]), u8::from(hdcp_first_auth[1])]);
        hasher.write(&[*hdmi_to_state]);
        hasher.finish()
    }
}

/// Replies staged while one send's packets execute. The AV manager
/// keeps two staging buffers and commits the plain one before the
/// system-controller one; events go straight to the stream after
/// both. The overflow checks every command makes look at the plain
/// buffer only.
#[derive(Debug, Default)]
struct ReplyBatch {
    plain: Vec<u8>,
    syscon: Vec<u8>,
    events: Vec<u8>,
    /// Bytes a full staging buffer refused.
    dropped: u64,
}

impl ReplyBatch {
    fn free(&self) -> usize {
        av::PS3AV_RX_BUF_SIZE.saturating_sub(self.plain.len())
    }

    fn refuse_if_full(&mut self, cid: u32, body_len: usize) -> bool {
        if self.free() < av::PS3AV_REPLY_HEADER_LEN + body_len {
            self.reply(false, cid, av::PS3AV_STATUS_BUFFER_OVERFLOW, &[]);
            true
        } else {
            false
        }
    }

    /// Stage a reply header (`version, length, cid | REPLY, status`)
    /// and body into the plain or system-controller buffer.
    fn reply(&mut self, syscon: bool, cid: u32, status: u32, body: &[u8]) {
        let total = av::PS3AV_REPLY_HEADER_LEN + body.len();
        let target = if syscon {
            &mut self.syscon
        } else {
            &mut self.plain
        };
        if target.len() + total > av::PS3AV_RX_BUF_SIZE {
            self.dropped += total as u64;
            return;
        }
        target.extend_from_slice(&av::PS3AV_VERSION.to_be_bytes());
        target.extend_from_slice(&((body.len() + 8) as u16).to_be_bytes());
        target.extend_from_slice(&(cid | av::PS3AV_REPLY_BIT).to_be_bytes());
        target.extend_from_slice(&status.to_be_bytes());
        target.extend_from_slice(body);
    }
}

fn rd16(p: &[u8], off: usize) -> u16 {
    u16::from_be_bytes([p[off], p[off + 1]])
}

fn rd32(p: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([p[off], p[off + 1], p[off + 2], p[off + 3]])
}

/// A packet's bytes, zero-padded to the length its header declares
/// so field reads past a short send read zeros rather than fault.
fn padded(tx: &[u8], off: usize, len: usize) -> Vec<u8> {
    let mut pkt = vec![0u8; len];
    let end = off.saturating_add(len).min(tx.len());
    if off < end {
        pkt[..end - off].copy_from_slice(&tx[off..end]);
    }
    pkt
}

/// How the parser checks a command's size before running it.
#[derive(Clone, Copy)]
enum SizeRule {
    /// The packet must be exactly this long, header included.
    Exact(usize),
    Unchecked,
    Computed(fn(&[u8]) -> usize),
}

type CidHandler = fn(&mut Lv2Host, u32, &[u8], &mut ReplyBatch);

/// One row per command the AV manager answers: the size rule the
/// parser applies and the handler that runs when it passes.
struct CidSpec {
    cid: u32,
    size: SizeRule,
    run: CidHandler,
}

const CID_TABLE: &[CidSpec] = &[
    CidSpec {
        cid: av::PS3AV_CID_AV_INIT,
        size: SizeRule::Exact(av::PS3AV_PKT_AV_INIT_LEN),
        run: Lv2Host::cid_av_init,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_FIN,
        size: SizeRule::Exact(av::PS3AV_HEADER_LEN),
        run: Lv2Host::cid_av_fin,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_GET_HW_CONF,
        size: SizeRule::Exact(av::PS3AV_HEADER_LEN),
        run: Lv2Host::cid_get_hw_conf,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_GET_MONITOR_INFO,
        size: SizeRule::Exact(av::PS3AV_PKT_GET_MONITOR_INFO_LEN),
        run: Lv2Host::cid_get_monitor_info,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_GET_BKSV_LIST,
        size: SizeRule::Exact(av::PS3AV_PKT_GET_BKSV_LEN),
        run: Lv2Host::cid_get_bksv_list,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_ENABLE_EVENT,
        size: SizeRule::Exact(av::PS3AV_PKT_ENABLE_EVENT_LEN),
        run: Lv2Host::cid_enable_event,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_DISABLE_EVENT,
        size: SizeRule::Exact(av::PS3AV_PKT_ENABLE_EVENT_LEN),
        run: Lv2Host::cid_disable_event,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_TV_MUTE,
        size: SizeRule::Exact(av::PS3AV_PKT_AV_AUDIO_MUTE_LEN),
        run: Lv2Host::cid_ack,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_NULL_CMD,
        size: SizeRule::Exact(av::PS3AV_PKT_NULL_CMD_LEN),
        run: Lv2Host::cid_ack,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_GET_AKSV,
        size: SizeRule::Exact(av::PS3AV_HEADER_LEN),
        run: Lv2Host::cid_get_aksv,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_VIDEO_DISABLE_SIG,
        size: SizeRule::Exact(av::PS3AV_PKT_VIDEO_DISABLE_SIG_LEN),
        run: Lv2Host::cid_video_disable_sig,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_VIDEO_YTRAPCONTROL,
        size: SizeRule::Exact(av::PS3AV_PKT_YTRAPCONTROL_LEN),
        run: Lv2Host::cid_video_ytrapcontrol,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_AUDIO_MUTE,
        size: SizeRule::Exact(av::PS3AV_PKT_AV_AUDIO_MUTE_LEN),
        run: Lv2Host::cid_av_audio_mute,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_ACP_CTRL,
        size: SizeRule::Exact(av::PS3AV_PKT_ACP_CTRL_LEN),
        run: Lv2Host::cid_acp_ctrl,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_SET_ACP_PACKET,
        size: SizeRule::Exact(av::PS3AV_PKT_SET_ACP_PACKET_LEN),
        run: Lv2Host::cid_set_acp_packet,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_ADD_SIGNAL_CTL,
        size: SizeRule::Exact(av::PS3AV_PKT_ADD_SIGNAL_CTL_LEN),
        run: Lv2Host::cid_avmulti_only,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_SET_CGMS_WSS,
        size: SizeRule::Exact(av::PS3AV_PKT_SET_CGMS_WSS_LEN),
        run: Lv2Host::cid_avmulti_only,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_HDMI_MODE,
        size: SizeRule::Exact(av::PS3AV_PKT_SET_HDMI_MODE_LEN),
        run: Lv2Host::cid_hdmi_mode,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_CEC_MESSAGE,
        size: SizeRule::Unchecked,
        run: Lv2Host::cid_blind_ack,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_GET_CEC_CONFIG,
        size: SizeRule::Exact(av::PS3AV_HEADER_LEN),
        run: Lv2Host::cid_get_cec_config,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_UNK11,
        size: SizeRule::Unchecked,
        run: Lv2Host::cid_blind_ack,
    },
    CidSpec {
        cid: av::PS3AV_CID_AV_UNK12,
        size: SizeRule::Unchecked,
        run: Lv2Host::cid_blind_ack,
    },
    CidSpec {
        cid: av::PS3AV_CID_VIDEO_INIT,
        size: SizeRule::Exact(av::PS3AV_HEADER_LEN),
        run: Lv2Host::cid_ack,
    },
    CidSpec {
        cid: av::PS3AV_CID_VIDEO_ROUTE,
        size: SizeRule::Exact(av::PS3AV_PKT_VIDEO_ROUTE_LEN),
        run: Lv2Host::cid_video_route,
    },
    CidSpec {
        cid: av::PS3AV_CID_VIDEO_FORMAT,
        size: SizeRule::Exact(av::PS3AV_PKT_VIDEO_FORMAT_LEN),
        run: Lv2Host::cid_video_format,
    },
    CidSpec {
        cid: av::PS3AV_CID_VIDEO_PITCH,
        size: SizeRule::Exact(av::PS3AV_PKT_VIDEO_PITCH_LEN),
        run: Lv2Host::cid_video_pitch,
    },
    CidSpec {
        cid: av::PS3AV_CID_VIDEO_GET_HW_CONF,
        size: SizeRule::Exact(av::PS3AV_HEADER_LEN),
        run: Lv2Host::cid_video_get_hw_conf,
    },
    CidSpec {
        cid: av::PS3AV_CID_AUDIO_INIT,
        size: SizeRule::Exact(av::PS3AV_HEADER_LEN),
        run: Lv2Host::cid_ack,
    },
    CidSpec {
        cid: av::PS3AV_CID_AUDIO_MODE,
        size: SizeRule::Exact(av::PS3AV_PKT_AUDIO_MODE_LEN),
        run: Lv2Host::cid_audio_mode,
    },
    CidSpec {
        cid: av::PS3AV_CID_AUDIO_MUTE,
        size: SizeRule::Unchecked,
        run: Lv2Host::cid_ack,
    },
    CidSpec {
        cid: av::PS3AV_CID_AUDIO_ACTIVE,
        size: SizeRule::Exact(av::PS3AV_PKT_AUDIO_SET_ACTIVE_LEN),
        run: Lv2Host::cid_ack,
    },
    CidSpec {
        cid: av::PS3AV_CID_AUDIO_INACTIVE,
        size: SizeRule::Exact(av::PS3AV_PKT_AUDIO_SET_ACTIVE_LEN),
        run: Lv2Host::cid_ack,
    },
    CidSpec {
        cid: av::PS3AV_CID_AUDIO_SPDIF_BIT,
        size: SizeRule::Exact(av::PS3AV_PKT_AUDIO_SPDIF_BIT_LEN),
        run: Lv2Host::cid_ack,
    },
    CidSpec {
        cid: av::PS3AV_CID_AUDIO_CTRL,
        size: SizeRule::Exact(av::PS3AV_PKT_AUDIO_CTRL_LEN),
        run: Lv2Host::cid_ack,
    },
    CidSpec {
        cid: av::PS3AV_CID_AVB_PARAM,
        size: SizeRule::Computed(inc_avset_size),
        run: Lv2Host::uart_inc_avset,
    },
];

fn cid_spec(cid: u32) -> Option<&'static CidSpec> {
    CID_TABLE.iter().find(|spec| spec.cid == cid)
}

/// A size no packet can declare; a computed rule returns it to refuse.
const IMPOSSIBLE_PKT_SIZE: usize = usize::MAX;

/// Size of an AVB_PARAM packet from its sub-packet counts and each
/// sub-packet's own header. A packet too short to hold the counts, an
/// over-count, or a sub-packet header past the declared end reads as
/// an impossible size so the parser refuses it.
///
/// What the system controller does with an over-count has no witness
/// here.
fn inc_avset_size(pkt: &[u8]) -> usize {
    if pkt.len() < av::PS3AV_PKT_INC_AVSET_LEN {
        return IMPOSSIBLE_PKT_SIZE;
    }
    let num_video = usize::from(rd16(pkt, 8));
    let num_av_video = usize::from(rd16(pkt, 12));
    let num_av_audio = usize::from(rd16(pkt, 14));
    if num_video > 2 || num_av_video > 4 || num_av_audio > 4 {
        return IMPOSSIBLE_PKT_SIZE;
    }
    let mut size = av::PS3AV_PKT_INC_AVSET_LEN;
    for _ in 0..num_video + num_av_video + num_av_audio {
        if size + 4 > pkt.len() {
            return IMPOSSIBLE_PKT_SIZE;
        }
        size += usize::from(rd16(pkt, size + 2)) + 4;
    }
    size
}

/// Video-mode bounds table indexed by the vid map below:
/// `(width_div, width, height)`.
const VIDEO_SCE_PARAMS: [(u32, u32, u32); 28] = [
    (0, 0, 0),
    (4, 2880, 480),
    (4, 2880, 480),
    (4, 2880, 576),
    (4, 2880, 576),
    (2, 1440, 480),
    (2, 1440, 576),
    (1, 1920, 1080),
    (1, 1920, 1080),
    (1, 1920, 1080),
    (1, 1280, 720),
    (1, 1280, 720),
    (1, 1920, 1080),
    (1, 1920, 1080),
    (1, 1920, 1080),
    (1, 1920, 1080),
    (1, 1280, 768),
    (1, 1280, 1024),
    (1, 1920, 1200),
    (1, 1360, 768),
    (1, 1280, 1470),
    (1, 1280, 1470),
    (1, 1920, 1080),
    (1, 1920, 2205),
    (1, 1920, 2205),
    (1, 1280, 721),
    (1, 720, 481),
    (1, 720, 577),
];

fn video_sce_index(vid: u32) -> Option<usize> {
    Some(match vid {
        16 => 1,
        1 => 2,
        3 | 17 => 4,
        5 => 5,
        6 => 6,
        18 => 7,
        7 | 33 => 8,
        8 | 34 => 9,
        9 | 31 => 10,
        10 | 32 => 11,
        11 | 35 | 37 => 12,
        12 | 36 | 38 => 13,
        19 => 14,
        20 => 15,
        13 => 16,
        14 => 17,
        15 => 18,
        21 => 19,
        22 | 27 => 20,
        23 | 28 => 21,
        24 => 22,
        25 | 29 => 23,
        26 | 30 => 24,
        39 => 25,
        40 => 26,
        41 => 27,
        _ => return None,
    })
}

/// Validate one `ps3av_pkt_video_mode`.
///
/// The system controller decides the accepted set, and its firmware is
/// not in dev_flash. The bounds table above, the `0x1CE07`
/// `out_format` mask, and the `unk2` and `pitch` limits have no
/// witness, so a rejection here is a guess.
fn video_mode_status(v: &[u8]) -> u32 {
    let head = rd32(v, 8);
    let unk2 = rd16(v, 14);
    let vid = rd32(v, 16);
    let width = rd32(v, 20);
    let height = rd32(v, 24);
    let pitch = rd32(v, 28);
    let out_format = rd32(v, 32);
    let format = rd32(v, 36);
    let order = rd16(v, 42);
    let Some(idx) = video_sce_index(vid) else {
        return av::PS3AV_STATUS_INVALID_VIDEO_PARAM;
    };
    let (width_div, max_width, max_height) = VIDEO_SCE_PARAMS[idx];
    let bad = head > av::PS3AV_HEAD_B_ANALOG
        || order > 1
        || format > 16
        || out_format > 16
        || (1u64 << out_format) & 0x1CE07 == 0
        || unk2 > 3
        || pitch & 7 != 0
        || pitch > u32::from(u16::MAX)
        || (width != 1280 && (width & 7 != 0 || width > u32::from(u16::MAX)))
        || (max_width != 720 && width > max_width / width_div)
        || !((height == 1470 && matches!(max_height, 721 | 481 | 577))
            || (height <= max_height && height <= u32::from(u16::MAX)));
    if bad {
        av::PS3AV_STATUS_INVALID_VIDEO_PARAM
    } else {
        av::PS3AV_STATUS_SUCCESS
    }
}

/// `monitor_name`, NUL-padded into the descriptor's 16-byte field.
const MONITOR_NAME: &[u8] = b"CellGov HDMI";

/// The name field runs from offset 12 to the `res_60` bits at 28.
const MONITOR_NAME_FIELD_LEN: usize = 16;
const _: () = assert!(MONITOR_NAME.len() <= MONITOR_NAME_FIELD_LEN);

/// The fixed HDMI 0 monitor: a 27-inch 16:9 1080p panel.
///
/// CellGov synthesises these bytes; on a console they come from the
/// EDID of the attached display. The descriptor names an ordinary
/// HDTV: the CEA modes and nothing else, 1080p60 native, Rec.709
/// colour. An EDID pass-through behaviour mode reports only the
/// monitor type.
fn hdmi_monitor_info(avport: u8, behavior: u8) -> [u8; av::PS3AV_MONITOR_INFO_LEN] {
    let mut m = [0u8; av::PS3AV_MONITOR_INFO_LEN];
    if behavior != av::PS3AV_HDMI_BEHAVIOR_NORMAL
        && behavior & av::PS3AV_HDMI_BEHAVIOR_EDID_PASS != 0
    {
        m[11] = if behavior & av::PS3AV_HDMI_BEHAVIOR_DVI != 0 {
            av::PS3AV_MONITOR_TYPE_DVI
        } else {
            av::PS3AV_MONITOR_TYPE_HDMI
        };
        return m;
    }
    m[0] = avport;
    m[1..11].copy_from_slice(&[0x4A, 0x13, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x15]);
    m[11] = av::PS3AV_MONITOR_TYPE_HDMI;
    m[12..12 + MONITOR_NAME.len()].copy_from_slice(MONITOR_NAME);
    // The CEA modes an HDTV carries, per refresh table. 480p and 576p
    // share a bit position, read against whichever table holds it.
    let hd = av::PS3AV_RESBIT_1280X720P | av::PS3AV_RESBIT_1920X1080I | av::PS3AV_RESBIT_1920X1080P;
    let native = av::PS3AV_RESBIT_1920X1080P;
    // res_60, res_50, res_other, res_vesa: (res_bits, native) each.
    // The native timing is 60 Hz, so the 50 Hz table lists modes with
    // no native entry; a TV carries no VESA mode.
    for (i, (bits, nat)) in [
        (av::PS3AV_RESBIT_720X480P | hd, native),
        (av::PS3AV_RESBIT_720X576P | hd, 0),
        (0, 0),
        (0, 0),
    ]
    .into_iter()
    .enumerate()
    {
        let at = 28 + i * 8;
        m[at..at + 4].copy_from_slice(&bits.to_be_bytes());
        m[at + 4..at + 8].copy_from_slice(&nat.to_be_bytes());
    }
    m[60] = av::PS3AV_CS_SUPPORTED
        | av::PS3AV_RGB_SELECTABLE_QUANTIZATION_RANGE
        | av::PS3AV_12BIT_COLOR;
    m[61] = av::PS3AV_CS_SUPPORTED | av::PS3AV_12BIT_COLOR;
    m[62] = av::PS3AV_CS_SUPPORTED;
    m[63] = av::PS3AV_COLORIMETRY_XVYCC_601
        | av::PS3AV_COLORIMETRY_XVYCC_709
        | av::PS3AV_COLORIMETRY_MD0
        | av::PS3AV_COLORIMETRY_MD1
        | av::PS3AV_COLORIMETRY_MD2;
    // Rec.709 primaries and a D65 white point, as 10-bit fractions of
    // the CIE x/y unit square: red (0.640, 0.330), green (0.300,
    // 0.600), blue (0.150, 0.060), white (0.3127, 0.3290).
    for (i, v) in [655u16, 338, 307, 614, 154, 61, 320, 337]
        .into_iter()
        .enumerate()
    {
        m[64 + i * 2..66 + i * 2].copy_from_slice(&v.to_be_bytes());
    }
    m[80..84].copy_from_slice(&100u32.to_be_bytes());
    m[84] = 1; // supported_ai
    m[85] = 0x4F; // speaker_info
    m[86..88].copy_from_slice(&7u16.to_be_bytes()); // num_of_audio_block
    let audio: [(u8, u8, u8, u8); 7] = [
        (av::PS3AV_MON_INFO_AUDIO_TYPE_LPCM, 8, 0x7F, 0x07),
        (av::PS3AV_MON_INFO_AUDIO_TYPE_AC3, 8, 0x7F, 0xFF),
        (av::PS3AV_MON_INFO_AUDIO_TYPE_AAC, 8, 0x7F, 0xFF),
        (av::PS3AV_MON_INFO_AUDIO_TYPE_DTS, 8, 0x7F, 0xFF),
        (av::PS3AV_MON_INFO_AUDIO_TYPE_DDP, 8, 0x7F, 0xFF),
        (av::PS3AV_MON_INFO_AUDIO_TYPE_DTS_HD, 8, 0x7F, 0xFF),
        (av::PS3AV_MON_INFO_AUDIO_TYPE_DOLBY_THD, 8, 0x7F, 0xFF),
    ];
    for (i, (ty, ch, fs, sbit)) in audio.into_iter().enumerate() {
        let at = 88 + i * 4;
        m[at..at + 4].copy_from_slice(&[ty, ch, fs, sbit]);
    }
    // 60 cm by 34 cm: a 27-inch panel at 16:9.
    m[152..154].copy_from_slice(&60u16.to_be_bytes()); // hor_screen_size
    m[154..156].copy_from_slice(&34u16.to_be_bytes()); // ver_screen_size
    m[156] = 0b1111; // supported_content_types

    // The five 3D resolution blocks at 160..200 stay zero: a plain
    // HDTV carries no stereoscopic timing.
    m
}

/// The AV-multi port: every resolution, all three colour spaces, one
/// stereo LPCM block.
fn avmulti_monitor_info() -> [u8; av::PS3AV_MONITOR_INFO_LEN] {
    let mut m = [0u8; av::PS3AV_MONITOR_INFO_LEN];
    m[0] = av::PS3AV_AVPORT_AVMULTI_0 as u8;
    m[11] = av::PS3AV_MONITOR_TYPE_AVMULTI;
    for at in [28, 36, 52] {
        m[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    }
    m[60] = av::PS3AV_CS_SUPPORTED;
    m[61] = av::PS3AV_CS_SUPPORTED;
    m[62] = av::PS3AV_CS_SUPPORTED;
    m[85] = 1; // speaker_info
    m[86..88].copy_from_slice(&1u16.to_be_bytes());
    m[88..92].copy_from_slice(&[av::PS3AV_MON_INFO_AUDIO_TYPE_LPCM, 2, 127, 7]);
    m
}

/// HDCP key list body: `(ksv_cnt u32, ksv[cnt][5])` padded to four
/// bytes, or a bare zero count when HDCP is off.
fn ksv_list_body(behavior: u8) -> Vec<u8> {
    let mut body = Vec::new();
    if behavior == av::PS3AV_HDMI_BEHAVIOR_NORMAL
        || behavior & av::PS3AV_HDMI_BEHAVIOR_HDCP_OFF == 0
    {
        body.extend_from_slice(&1u32.to_be_bytes());
        body.extend_from_slice(&av::PS3AV_BKSV_VALUE);
        while body.len() % 4 != 0 {
            body.push(0);
        }
    } else {
        body.extend_from_slice(&0u32.to_be_bytes());
    }
    body
}

impl Lv2Host {
    /// `sys_uart_initialize` (367).
    ///
    /// # Errors
    ///
    /// - `CELL_ENOSYS` for a caller without root privilege.
    /// - `CELL_EPERM` once the UART is already claimed.
    pub(super) fn dispatch_uart_initialize(&mut self) -> Lv2Dispatch {
        if !self.has_root_perm() {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOSYS.into());
        }
        if self.state.uart.initialized {
            return Lv2Dispatch::immediate(cell_errors::CELL_EPERM.into());
        }
        self.state.uart.initialized = true;
        Lv2Dispatch::immediate(0)
    }

    /// `sys_uart_get_params` (370): the two ring sizes, u64 BE each.
    ///
    /// # Errors
    ///
    /// - `CELL_ENOSYS` without root privilege.
    /// - `CELL_ESRCH` before `sys_uart_initialize`.
    /// - `CELL_EFAULT` for an unwritable output block.
    pub(super) fn dispatch_uart_get_params(
        &mut self,
        params_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if !self.has_root_perm() {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOSYS.into());
        }
        if !self.state.uart.initialized {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        }
        if params_ptr == 0 || !rt.writable(u64::from(params_ptr), av::SYS_UART_PARAMS_LEN) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        let mut out = [0u8; av::SYS_UART_PARAMS_LEN];
        out[..8].copy_from_slice(&(av::PS3AV_RX_BUF_SIZE as u64).to_be_bytes());
        out[8..].copy_from_slice(&(av::PS3AV_TX_BUF_SIZE as u64).to_be_bytes());
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![Effect::SharedWriteIntent {
                range: ByteRange::contiguous_u32(params_ptr, av::SYS_UART_PARAMS_LEN as u32),
                bytes: WritePayload::from_slice(&out),
                ordering: PriorityClass::Normal,
                source: requester,
                source_time: tick,
            }],
        }
    }

    /// `sys_uart_receive` (368): pops up to `size` bytes of the reply
    /// stream into `buf_ptr` and returns the count. An empty stream
    /// returns 0 in non-blocking mode and parks a blocking caller
    /// behind any readers already parked, until sends stage enough
    /// bytes to reach it.
    ///
    /// The kernel's non-blocking arm answers `CELL_EBUSY` only while
    /// another reader holds the receive lock mid-copy; a dispatch
    /// here is atomic, so that window does not exist and no arm
    /// returns `CELL_EBUSY`.
    ///
    /// # Errors
    ///
    /// - `CELL_ENOSYS` without root privilege.
    /// - `CELL_EINVAL` for a mode other than 0 / 1, or a transfer over
    ///   [`av::SYS_UART_MAX_TRANSFER`], which draws a named break.
    /// - `CELL_ESRCH` before `sys_uart_initialize`, or for a caller
    ///   with no PPU thread record.
    /// - `CELL_EFAULT` for an unwritable buffer, checked before any
    ///   byte leaves the stream.
    pub(super) fn dispatch_uart_receive(
        &mut self,
        buf_ptr: u32,
        size: u64,
        mode: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if !self.has_root_perm() {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOSYS.into());
        }
        if size == 0 {
            return Lv2Dispatch::immediate(0);
        }
        if mode & !(av::SYS_UART_MODE_BLOCKING_BIG_OP | av::SYS_UART_MODE_NOT_BLOCKING_BIG_OP) != 0
        {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        if !self.state.uart.initialized {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        }
        if size > av::SYS_UART_MAX_TRANSFER {
            self.log_invariant_break(
                "dispatch.uart_transfer_over_cap",
                format_args!(
                    "sys_uart_receive: {size} bytes exceeds the {} byte transfer cap; returning CELL_EINVAL",
                    av::SYS_UART_MAX_TRANSFER
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        let available = self.state.uart.rx.len();
        if available == 0 {
            if mode != av::SYS_UART_MODE_BLOCKING_BIG_OP {
                return Lv2Dispatch::immediate(0);
            }
            let Some(thread) = self.state.ppu_threads.thread_id_for_unit(requester) else {
                return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
            };
            if !rt.writable(
                u64::from(buf_ptr),
                size.min(av::PS3AV_RX_BUF_SIZE as u64) as usize,
            ) {
                return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
            }
            if self.state.uart.readers.iter().any(|r| r.thread == thread) {
                // A parked thread cannot dispatch; two records for
                // one thread would wake it twice.
                self.log_invariant_break(
                    "dispatch.uart_reader_reparked",
                    format_args!(
                        "sys_uart_receive: {thread:?} is already parked on the reply stream; \
                         returning CELL_ESRCH"
                    ),
                );
                return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
            }
            if !self.state.uart.readers.is_empty() {
                self.obs.uart_readers_queued += 1;
            }
            self.state.uart.readers.push_back(UartReader {
                thread,
                buf_ptr,
                size,
            });
            return Lv2Dispatch::Block {
                reason: Lv2BlockReason::Uart,
                pending: PendingResponse::ReturnCode { code: 0 },
                effects: vec![],
            };
        }
        let n = (size as usize).min(available);
        if !rt.writable(u64::from(buf_ptr), n) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        let bytes: Vec<u8> = self.state.uart.rx.drain(..n).collect();
        Lv2Dispatch::Immediate {
            code: n as u64,
            effects: vec![Effect::SharedWriteIntent {
                range: ByteRange::contiguous_u32(buf_ptr, n as u32),
                bytes: WritePayload::from_slice(&bytes),
                ordering: PriorityClass::Normal,
                source: requester,
                source_time: tick,
            }],
        }
    }

    /// `sys_uart_send` (369): parses every packet in the buffer,
    /// stages the replies and any events they trigger, and hands the
    /// stream to the parked readers in park order, each taking up to
    /// its own size while bytes remain.
    ///
    /// # Errors
    ///
    /// - `CELL_ENOSYS` without root privilege.
    /// - `CELL_EINVAL` for a mode above 3, or a transfer over
    ///   [`av::SYS_UART_MAX_TRANSFER`], which draws a named break.
    /// - `CELL_ESRCH` before `sys_uart_initialize`.
    /// - `CELL_EFAULT` for an unreadable buffer.
    /// - `CELL_EAGAIN` in mode 2 when the buffer exceeds the TX ring.
    pub(super) fn dispatch_uart_send(
        &mut self,
        buf_ptr: u32,
        size: u64,
        mode: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if !self.has_root_perm() {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOSYS.into());
        }
        if size == 0 {
            return Lv2Dispatch::immediate(0);
        }
        if mode
            & !(av::SYS_UART_MODE_BLOCKING_BIG_OP
                | av::SYS_UART_MODE_NOT_BLOCKING_OP
                | av::SYS_UART_MODE_NOT_BLOCKING_BIG_OP)
            != 0
        {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        if !self.state.uart.initialized {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        }
        if size > av::SYS_UART_MAX_TRANSFER {
            self.log_invariant_break(
                "dispatch.uart_transfer_over_cap",
                format_args!(
                    "sys_uart_send: {size} bytes exceeds the {} byte transfer cap; returning CELL_EINVAL",
                    av::SYS_UART_MAX_TRANSFER
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        let Some(tx) = rt.read_committed(u64::from(buf_ptr), size as usize) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };
        if mode == av::SYS_UART_MODE_NOT_BLOCKING_OP && size > av::PS3AV_TX_BUF_SIZE as u64 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EAGAIN.into());
        }
        // Mode 0 pushes its first chunk and, when the ring cannot take
        // that chunk whole, reports the chunk's size rather than the
        // ring's; the bytes past the ring are dropped either way.
        let sent = if mode == av::SYS_UART_MODE_NOT_BLOCKING_BIG_OP
            && size > av::PS3AV_TX_BUF_SIZE as u64
        {
            size.min(av::SYS_UART_CHUNK)
        } else {
            size
        };
        let tx = tx.to_vec();
        let mut batch = ReplyBatch::default();
        self.uart_parse(&tx, &mut batch);
        self.uart_commit(batch);
        let deliveries = self.uart_deliver_to_readers(requester, tick);
        if deliveries.is_empty() {
            return Lv2Dispatch::immediate(sent);
        }
        let mut woken_unit_ids = Vec::with_capacity(deliveries.len());
        let mut response_updates = Vec::with_capacity(deliveries.len());
        let mut effects = Vec::with_capacity(deliveries.len());
        for (unit, effect, code) in deliveries {
            woken_unit_ids.push(unit);
            response_updates.push((unit, PendingResponse::ReturnCode { code }));
            effects.push(effect);
        }
        Lv2Dispatch::WakeAndReturn {
            code: sent,
            woken_unit_ids,
            response_updates,
            effects,
        }
    }

    /// Append a batch to the reply stream: plain replies, then
    /// system-controller replies, then events.
    fn uart_commit(&mut self, batch: ReplyBatch) {
        self.obs.uart_rx_overflow_bytes += batch.dropped;
        for chunk in [batch.plain, batch.syscon, batch.events] {
            let room = av::PS3AV_RX_BUF_SIZE.saturating_sub(self.state.uart.rx.len());
            let take = chunk.len().min(room);
            self.obs.uart_rx_overflow_bytes += (chunk.len() - take) as u64;
            self.state.uart.rx.extend_from_slice(&chunk[..take]);
        }
    }

    /// Hand the stream to the parked readers, front first, while
    /// bytes remain: one `(unit, write, count)` per reader served.
    /// A reader whose thread record is gone is dropped from the
    /// queue (named break) without consuming bytes.
    fn uart_deliver_to_readers(
        &mut self,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Vec<(UnitId, Effect, u64)> {
        let mut served = Vec::new();
        while !self.state.uart.rx.is_empty() {
            let Some(reader) = self.state.uart.readers.pop_front() else {
                break;
            };
            let Some(unit) = self.resolve_wake_thread(reader.thread, "uart_deliver.reader") else {
                continue;
            };
            let n = (reader.size as usize).min(self.state.uart.rx.len());
            let bytes: Vec<u8> = self.state.uart.rx.drain(..n).collect();
            served.push((
                unit,
                Effect::SharedWriteIntent {
                    range: ByteRange::contiguous_u32(reader.buf_ptr, n as u32),
                    bytes: WritePayload::from_slice(&bytes),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: tick,
                },
                n as u64,
            ));
        }
        served
    }

    /// Walk the packets in one send.
    fn uart_parse(&mut self, tx: &[u8], batch: &mut ReplyBatch) {
        if tx.len() >= av::PS3AV_TX_BUF_SIZE {
            // An overfull ring draws one overflow reply, addressed
            // by the low half of the first cid.
            let cid = u32::from(rd16(&padded(tx, 0, 8), 6));
            batch.reply(false, cid, av::PS3AV_STATUS_BUFFER_OVERFLOW, &[]);
            return;
        }
        let mut off = 0;
        while off < tx.len() {
            let hdr = padded(tx, off, av::PS3AV_HEADER_LEN);
            let version = rd16(&hdr, 0);
            let length = rd16(&hdr, 2);
            let cid = rd32(&hdr, 4);
            // The AV manager sizes a packet in 16-bit arithmetic and
            // walks by that size however small it is; the poison
            // length is the one value that would walk zero bytes.
            let pkt_size = usize::from(length.wrapping_add(4));
            if length == av::PS3AV_LENGTH_POISON {
                batch.reply(
                    false,
                    av::PS3AV_CID_POISON_REPLY,
                    av::PS3AV_STATUS_FAILURE,
                    &[],
                );
                return;
            }
            if version != av::PS3AV_VERSION {
                batch.reply(false, cid & 0xFFFF, av::PS3AV_STATUS_INVALID_COMMAND, &[]);
                return;
            }
            // The handler's view always covers a header; the walk
            // does not.
            let pkt = padded(tx, off, pkt_size.max(av::PS3AV_HEADER_LEN));
            off += pkt_size;
            *self.obs.uart_cids.entry(cid).or_insert(0) += 1;
            let Some(spec) = cid_spec(cid) else {
                *self.obs.uart_unknown_cids.entry(cid).or_insert(0) += 1;
                self.log_invariant_break(
                    "dispatch.uart_unknown_cid",
                    format_args!(
                        "sys_uart_send: no AV-manager handler for cid 0x{cid:08x}; the guest gets no reply"
                    ),
                );
                continue;
            };
            let expected = match spec.size {
                SizeRule::Exact(n) => Some(n),
                SizeRule::Unchecked => None,
                SizeRule::Computed(f) => Some(f(&pkt)),
            };
            if let Some(expected) = expected {
                if expected != pkt_size {
                    batch.reply(
                        false,
                        cid & 0xFFFF,
                        av::PS3AV_STATUS_INVALID_SAMPLE_SIZE,
                        &[],
                    );
                    return;
                }
            }
            (spec.run)(self, cid, &pkt, batch);
        }
    }

    /// Acknowledge a command that carries no state.
    fn cid_ack(&mut self, cid: u32, _pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &[]);
    }

    /// Acknowledge a command nothing here models; the break names
    /// the fabricated success.
    fn cid_blind_ack(&mut self, cid: u32, _pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        self.log_invariant_break(
            "dispatch.uart_cid_acked_blind",
            format_args!("sys_uart_send: cid 0x{cid:08x} acknowledged with no model behind it"),
        );
        batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &[]);
    }

    fn cid_av_init(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        self.state.uart.av_cmd_ver = rd16(pkt, 0);
        let event_bit = rd32(pkt, 8);
        self.state.uart.hdmi_events |= event_bit;
        if event_bit & av::PS3AV_EVENT_BIT_UNK != 0 {
            batch.reply(
                false,
                cid,
                av::PS3AV_STATUS_SUCCESS,
                &[0; av::PS3AV_REPLY_AV_INIT_LEN],
            );
        } else {
            batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &[]);
        }
    }

    fn cid_av_fin(&mut self, cid: u32, _pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        self.state.uart.hdmi_events = 0;
        batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &[]);
    }

    /// One HDMI, one AV-multi, one S/PDIF, extra bitstreams.
    fn cid_get_hw_conf(&mut self, cid: u32, _pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, av::PS3AV_REPLY_GET_HW_CONF_LEN) {
            return;
        }
        batch.reply(
            false,
            cid,
            av::PS3AV_STATUS_SUCCESS,
            &[0, 1, 0, 1, 0, 1, 0, 1],
        );
    }

    fn cid_get_monitor_info(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        let avport = rd16(pkt, 8);
        if avport == av::PS3AV_AVPORT_AVMULTI_0 {
            batch.reply(
                false,
                cid,
                av::PS3AV_STATUS_SUCCESS,
                &avmulti_monitor_info(),
            );
        } else if avport <= av::PS3AV_AVPORT_HDMI_1 {
            if avport == av::PS3AV_AVPORT_HDMI_1 {
                batch.reply(true, cid, av::PS3AV_STATUS_SYSCON_COMMUNICATE_FAIL, &[]);
                return;
            }
            let info = hdmi_monitor_info(avport as u8, self.state.uart.hdmi_behavior);
            batch.reply(
                true,
                cid,
                av::PS3AV_STATUS_SUCCESS,
                &info[..av::PS3AV_MONITOR_INFO_HDMI_LEN],
            );
        } else {
            batch.reply(false, cid, av::PS3AV_STATUS_INVALID_PORT, &[]);
        }
    }

    fn cid_get_bksv_list(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        let avport = rd16(pkt, 8);
        if avport > av::PS3AV_AVPORT_HDMI_1 {
            batch.reply(false, cid, av::PS3AV_STATUS_INVALID_PORT, &[]);
            return;
        }
        if avport == av::PS3AV_AVPORT_HDMI_1 {
            batch.reply(true, cid, av::PS3AV_STATUS_SYSCON_COMMUNICATE_FAIL, &[]);
            return;
        }
        let mut body = Vec::with_capacity(16);
        body.extend_from_slice(&avport.to_be_bytes());
        body.extend_from_slice(&[0, 0]);
        body.extend_from_slice(&ksv_list_body(self.state.uart.hdmi_behavior));
        batch.reply(true, cid, av::PS3AV_STATUS_SUCCESS, &body);
    }

    fn cid_enable_event(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        self.state.uart.hdmi_events |= rd32(pkt, 8);
        batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &[]);
    }

    fn cid_disable_event(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        self.state.uart.hdmi_events &= !rd32(pkt, 8);
        batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &[]);
    }

    fn cid_get_aksv(&mut self, cid: u32, _pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, av::PS3AV_REPLY_GET_AKSV_LEN) {
            return;
        }
        let mut body = [0u8; av::PS3AV_REPLY_GET_AKSV_LEN];
        body[..4].copy_from_slice(&(av::PS3AV_AKSV_VALUE.len() as u32).to_be_bytes());
        body[4..9].copy_from_slice(&av::PS3AV_AKSV_VALUE);
        batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &body);
    }

    fn cid_video_disable_sig(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        let avport = rd16(pkt, 8);
        if avport <= av::PS3AV_AVPORT_HDMI_1 {
            if avport == av::PS3AV_AVPORT_HDMI_1 {
                batch.reply(true, cid, av::PS3AV_STATUS_SYSCON_COMMUNICATE_FAIL, &[]);
                return;
            }
            self.state.uart.hdmi_res_set[usize::from(avport)] = false;
            batch.reply(true, cid, av::PS3AV_STATUS_SUCCESS, &[]);
        } else if avport == av::PS3AV_AVPORT_AVMULTI_0 {
            // The reply comes only after head B is configured; before
            // that the command is silent.
            if self.state.uart.head_b_initialized {
                batch.reply(true, cid, av::PS3AV_STATUS_SUCCESS, &[]);
            }
        } else {
            batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &[]);
        }
    }

    fn cid_video_ytrapcontrol(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, av::PS3AV_REPLY_GET_HW_CONF_LEN) {
            return;
        }
        let unk1 = rd16(pkt, 8);
        if unk1 != 0 && unk1 != 5 {
            batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &[]);
        } else {
            batch.reply(true, cid, av::PS3AV_STATUS_SUCCESS, &[]);
        }
    }

    fn cid_av_audio_mute(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
        let avport = rd16(pkt, 8);
        if avport == av::PS3AV_AVPORT_AVMULTI_1 {
            batch.reply(false, cid, av::PS3AV_STATUS_INVALID_PORT, &[]);
        } else if (avport > av::PS3AV_AVPORT_HDMI_1 && avport != av::PS3AV_AVPORT_AVMULTI_0)
            || avport == av::PS3AV_AVPORT_HDMI_1
        {
            batch.reply(true, cid, av::PS3AV_STATUS_SYSCON_COMMUNICATE_FAIL, &[]);
        } else {
            batch.reply(true, cid, av::PS3AV_STATUS_SUCCESS, &[]);
        }
    }

    fn cid_acp_ctrl(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
        if u16::from(pkt[8]) > av::PS3AV_AVPORT_HDMI_0 {
            batch.reply(true, cid, av::PS3AV_STATUS_INVALID_PORT, &[]);
        } else {
            batch.reply(true, cid, av::PS3AV_STATUS_SUCCESS, &[]);
        }
    }

    fn cid_set_acp_packet(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
        let avport = u16::from(pkt[8]);
        let pkt_type = pkt[9];
        if avport > av::PS3AV_AVPORT_HDMI_0
            || (pkt_type > 0x0A && pkt_type < 0x81)
            || pkt_type > 0x85
        {
            batch.reply(true, cid, av::PS3AV_STATUS_INVALID_PORT, &[]);
        } else {
            batch.reply(true, cid, av::PS3AV_STATUS_SUCCESS, &[]);
        }
    }

    /// ADD_SIGNAL_CTL and SET_CGMS_WSS: analogue-only knobs.
    fn cid_avmulti_only(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        if rd16(pkt, 8) != av::PS3AV_AVPORT_AVMULTI_0 {
            batch.reply(false, cid, av::PS3AV_STATUS_INVALID_PORT, &[]);
        } else {
            batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &[]);
        }
    }

    fn cid_hdmi_mode(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
        let mode = pkt[8];
        if mode != av::PS3AV_HDMI_BEHAVIOR_NORMAL && mode & av::PS3AV_HDMI_BEHAVIOR_HDCP_OFF != 0 {
            batch.reply(true, cid, av::PS3AV_STATUS_UNSUPPORTED_HDMI_MODE, &[]);
            return;
        }
        self.state.uart.hdmi_behavior = mode;
        let last = if self.state.uart.hdmi_res_set[0] {
            HDMI_STATE_HDCP_DONE
        } else {
            HDMI_STATE_PLUGGED
        };
        self.uart_hdmi_script(HDMI_STATE_UNPLUGGED, last, batch);
        batch.reply(true, cid, av::PS3AV_STATUS_SUCCESS, &[]);
    }

    fn cid_get_cec_config(&mut self, cid: u32, _pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, av::PS3AV_REPLY_GET_CEC_CONFIG_LEN) {
            return;
        }
        batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &1u32.to_be_bytes());
    }

    fn cid_video_format(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        let head = rd32(pkt, 8);
        let format = rd32(pkt, 12);
        let order = rd16(pkt, 18);
        if head > av::PS3AV_HEAD_B_ANALOG || order > 1 || format > 16 {
            batch.reply(false, cid, av::PS3AV_STATUS_INVALID_VIDEO_PARAM, &[]);
        } else {
            batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &[]);
        }
    }

    /// Routing exists only for the PS2 graphics partition.
    fn cid_video_route(&mut self, cid: u32, _pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        batch.reply(false, cid, av::PS3AV_STATUS_NO_SEL, &[]);
    }

    fn cid_video_pitch(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        let head = rd32(pkt, 8);
        let pitch = rd32(pkt, 12);
        if head > av::PS3AV_HEAD_B_ANALOG || pitch & 7 != 0 || pitch > u32::from(u16::MAX) {
            batch.reply(false, cid, av::PS3AV_STATUS_INVALID_VIDEO_PARAM, &[]);
        } else {
            batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &[]);
        }
    }

    /// No PS2 graphics partition.
    fn cid_video_get_hw_conf(&mut self, cid: u32, _pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, av::PS3AV_REPLY_VIDEO_GET_HW_CONF_LEN) {
            return;
        }
        batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &0u32.to_be_bytes());
    }

    fn cid_audio_mode(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        let avport = u16::from(pkt[8]);
        let fs = rd32(pkt, 20);
        let port_ok = matches!(
            avport,
            av::PS3AV_AVPORT_HDMI_0
                | av::PS3AV_AVPORT_HDMI_1
                | av::PS3AV_AVPORT_AVMULTI_0
                | av::PS3AV_AVPORT_SPDIF_0
                | av::PS3AV_AVPORT_SPDIF_1
        );
        if !port_ok || fs > av::PS3AV_AUDIO_FS_192K {
            batch.reply(false, cid, av::PS3AV_STATUS_INVALID_AUDIO_PARAM, &[]);
        } else {
            batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &[]);
        }
    }

    /// AVB_PARAM: a video-mode section, an AV-video section, and an
    /// AV-audio section, each validated in turn.
    fn uart_inc_avset(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        let num_video = usize::from(rd16(pkt, 8));
        let num_av_video = usize::from(rd16(pkt, 12));
        let num_av_audio = usize::from(rd16(pkt, 14));
        let mut at = av::PS3AV_PKT_INC_AVSET_LEN;
        let sub = |at: usize| -> Vec<u8> {
            let len = usize::from(rd16(&padded(pkt, at, 4), 2)) + 4;
            padded(pkt, at, len.max(av::PS3AV_PKT_VIDEO_MODE_LEN))
        };

        let mut video_status = av::PS3AV_STATUS_SUCCESS;
        for _ in 0..num_video {
            let v = sub(at);
            let status = video_mode_status(&v);
            if rd32(&v, 8) == av::PS3AV_HEAD_B_ANALOG {
                self.state.uart.head_b_initialized = true;
            }
            if status != av::PS3AV_STATUS_SUCCESS {
                video_status = status;
            }
            at += usize::from(rd16(&v, 2)) + 4;
        }
        if num_av_video == 0 && num_av_audio == 0 {
            batch.reply(false, cid, video_status, &[]);
            return;
        }

        let mut syscon_ok = true;
        let mut hdcp_done = [false; 2];
        for _ in 0..num_av_video {
            let v = sub(at);
            let avport = rd16(&v, 8);
            let av_vid = rd16(&v, 10);
            if avport <= av::PS3AV_AVPORT_HDMI_1 {
                if av_vid > 23 {
                    batch.reply(false, cid, av::PS3AV_STATUS_INVALID_AV_PARAM, &[]);
                    return;
                }
                if avport == av::PS3AV_AVPORT_HDMI_1 {
                    syscon_ok = false;
                } else if syscon_ok {
                    hdcp_done[usize::from(avport)] = true;
                }
            } else {
                if (avport != av::PS3AV_AVPORT_AVMULTI_0 && avport != av::PS3AV_AVPORT_AVMULTI_1)
                    || av_vid > 23
                    || (av_vid > 12 && av_vid != 18)
                {
                    batch.reply(false, cid, av::PS3AV_STATUS_INVALID_AV_PARAM, &[]);
                    return;
                }
                if avport == av::PS3AV_AVPORT_AVMULTI_1 {
                    syscon_ok = false;
                }
            }
            at += usize::from(rd16(&v, 2)) + 4;
        }
        self.state.uart.hdmi_res_set = hdcp_done;
        if hdcp_done[0] {
            self.uart_hdmi_script(HDMI_STATE_HDCP_DONE, HDMI_STATE_HDCP_DONE, batch);
        }

        let mut valid_av_audio = false;
        for _ in 0..num_av_audio {
            let a = sub(at);
            let avport = rd16(&a, 8);
            if avport <= av::PS3AV_AVPORT_HDMI_1 {
                valid_av_audio = true;
                if !syscon_ok || avport == av::PS3AV_AVPORT_HDMI_1 {
                    syscon_ok = false;
                    break;
                }
            }
            at += usize::from(rd16(&a, 2)) + 4;
        }

        if num_av_video > 0 || valid_av_audio {
            if !syscon_ok {
                batch.reply(true, cid, av::PS3AV_STATUS_SYSCON_COMMUNICATE_FAIL, &[]);
            } else {
                batch.reply(true, cid, av::PS3AV_STATUS_SUCCESS, &[]);
            }
        } else {
            batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &[]);
        }
    }

    /// Drive the HDMI 0 link from one step below `first` (or the last
    /// scripted state, whichever is lower) up to `last`, staging one
    /// event per step the guest has enabled.
    fn uart_hdmi_script(&mut self, first: u8, last: u8, batch: &mut ReplyBatch) {
        let base = (first - 1).min(self.state.uart.hdmi_to_state);
        self.state.uart.hdmi_to_state = last;
        for state in base + 1..=last {
            self.uart_hdmi_event(state, batch);
        }
    }

    fn uart_hdmi_event(&mut self, state: u8, batch: &mut ReplyBatch) {
        let ver = self.state.uart.av_cmd_ver;
        let enabled = self.state.uart.hdmi_events;
        let header = |cid: u32, length: u16| -> Vec<u8> {
            let mut p = Vec::with_capacity(av::PS3AV_HEADER_LEN);
            p.extend_from_slice(&ver.to_be_bytes());
            p.extend_from_slice(&length.to_be_bytes());
            p.extend_from_slice(&cid.to_be_bytes());
            p
        };
        match state {
            HDMI_STATE_UNPLUGGED => {
                if enabled & av::PS3AV_EVENT_BIT_UNPLUGGED == 0 {
                    self.obs.uart_events_gated += 1;
                    return;
                }
                self.state.uart.hdcp_first_auth[0] = true;
                batch
                    .events
                    .extend_from_slice(&header(av::PS3AV_CID_EVENT_UNPLUGGED, 4));
            }
            HDMI_STATE_PLUGGED => {
                if enabled & av::PS3AV_EVENT_BIT_PLUGGED == 0 {
                    self.obs.uart_events_gated += 1;
                    return;
                }
                let info = hdmi_monitor_info(0, self.state.uart.hdmi_behavior);
                let mut p = header(
                    av::PS3AV_CID_EVENT_PLUGGED,
                    av::PS3AV_MONITOR_INFO_LEN as u16,
                );
                p.extend_from_slice(&info[..av::PS3AV_MONITOR_INFO_HDMI_LEN]);
                batch.events.extend_from_slice(&p);
            }
            HDMI_STATE_HDCP_DONE => {
                let cid = if self.state.uart.hdcp_first_auth[0] {
                    if enabled & av::PS3AV_EVENT_BIT_HDCP_DONE == 0 {
                        self.obs.uart_events_gated += 1;
                        return;
                    }
                    self.state.uart.hdcp_first_auth[0] = false;
                    av::PS3AV_CID_EVENT_HDCP_DONE
                } else {
                    if enabled & av::PS3AV_EVENT_BIT_HDCP_REAUTH == 0 {
                        self.obs.uart_events_gated += 1;
                        return;
                    }
                    av::PS3AV_CID_EVENT_HDCP_REAUTH
                };
                let body = ksv_list_body(self.state.uart.hdmi_behavior);
                let mut p = header(cid, (av::PS3AV_HEADER_LEN + body.len() - 4) as u16);
                p.extend_from_slice(&body);
                batch.events.extend_from_slice(&p);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "tests/uart_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/uart_reader_queue_tests.rs"]
mod reader_queue_tests;

#[cfg(test)]
#[path = "tests/uart_purge_tests.rs"]
mod purge_tests;
