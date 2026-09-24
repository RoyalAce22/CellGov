//! The command table: each CID's size rule and handler.

use cellgov_ps3_abi::lv2::uart as av;

use crate::host::Lv2Host;

use super::packet::rd16;
use super::reply::ReplyBatch;

/// How the parser checks a command's size before running it.
#[derive(Clone, Copy)]
pub(super) enum SizeRule {
    /// The packet must be exactly this long, header included.
    Exact(usize),
    Unchecked,
    Computed(fn(&[u8]) -> usize),
}

pub(super) type CidHandler = fn(&mut Lv2Host, u32, &[u8], &mut ReplyBatch);

/// One row per command the AV manager answers: the size rule the
/// parser applies and the handler that runs when it passes.
pub(super) struct CidSpec {
    pub(super) cid: u32,
    pub(super) size: SizeRule,
    pub(super) run: CidHandler,
}

pub(super) const CID_TABLE: &[CidSpec] = &[
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

pub(super) fn cid_spec(cid: u32) -> Option<&'static CidSpec> {
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
