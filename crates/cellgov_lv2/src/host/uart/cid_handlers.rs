//! The per-CID command handlers.

use cellgov_ps3_abi::lv2::uart as av;

use crate::host::Lv2Host;

use super::av_data::{avmulti_monitor_info, hdmi_monitor_info, ksv_list_body};
use super::hdmi::{HDMI_STATE_HDCP_DONE, HDMI_STATE_PLUGGED, HDMI_STATE_UNPLUGGED};
use super::packet::{rd16, rd32};
use super::reply::ReplyBatch;

impl Lv2Host {
    /// Acknowledge a command that carries no state.
    pub(super) fn cid_ack(&mut self, cid: u32, _pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &[]);
    }

    /// Acknowledge a command nothing here models; the break names
    /// the fabricated success.
    pub(super) fn cid_blind_ack(&mut self, cid: u32, _pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        self.log_invariant_break(
            "dispatch.uart_cid_acked_blind",
            format_args!("sys_uart_send: cid 0x{cid:08x} acknowledged with no model behind it"),
        );
        batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &[]);
    }

    pub(super) fn cid_av_init(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
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

    pub(super) fn cid_av_fin(&mut self, cid: u32, _pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        self.state.uart.hdmi_events = 0;
        batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &[]);
    }

    /// One HDMI, one AV-multi, one S/PDIF, extra bitstreams.
    pub(super) fn cid_get_hw_conf(&mut self, cid: u32, _pkt: &[u8], batch: &mut ReplyBatch) {
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

    pub(super) fn cid_get_monitor_info(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
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

    pub(super) fn cid_get_bksv_list(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
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

    pub(super) fn cid_enable_event(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        self.state.uart.hdmi_events |= rd32(pkt, 8);
        batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &[]);
    }

    pub(super) fn cid_disable_event(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        self.state.uart.hdmi_events &= !rd32(pkt, 8);
        batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &[]);
    }

    pub(super) fn cid_get_aksv(&mut self, cid: u32, _pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, av::PS3AV_REPLY_GET_AKSV_LEN) {
            return;
        }
        let mut body = [0u8; av::PS3AV_REPLY_GET_AKSV_LEN];
        body[..4].copy_from_slice(&(av::PS3AV_AKSV_VALUE.len() as u32).to_be_bytes());
        body[4..9].copy_from_slice(&av::PS3AV_AKSV_VALUE);
        batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &body);
    }

    pub(super) fn cid_video_disable_sig(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
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

    pub(super) fn cid_video_ytrapcontrol(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
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

    pub(super) fn cid_av_audio_mute(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
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

    pub(super) fn cid_acp_ctrl(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
        if u16::from(pkt[8]) > av::PS3AV_AVPORT_HDMI_0 {
            batch.reply(true, cid, av::PS3AV_STATUS_INVALID_PORT, &[]);
        } else {
            batch.reply(true, cid, av::PS3AV_STATUS_SUCCESS, &[]);
        }
    }

    pub(super) fn cid_set_acp_packet(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
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
    pub(super) fn cid_avmulti_only(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        if rd16(pkt, 8) != av::PS3AV_AVPORT_AVMULTI_0 {
            batch.reply(false, cid, av::PS3AV_STATUS_INVALID_PORT, &[]);
        } else {
            batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &[]);
        }
    }

    pub(super) fn cid_hdmi_mode(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
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

    pub(super) fn cid_get_cec_config(&mut self, cid: u32, _pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, av::PS3AV_REPLY_GET_CEC_CONFIG_LEN) {
            return;
        }
        batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &1u32.to_be_bytes());
    }

    pub(super) fn cid_video_format(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
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
    pub(super) fn cid_video_route(&mut self, cid: u32, _pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, 0) {
            return;
        }
        batch.reply(false, cid, av::PS3AV_STATUS_NO_SEL, &[]);
    }

    pub(super) fn cid_video_pitch(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
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
    pub(super) fn cid_video_get_hw_conf(&mut self, cid: u32, _pkt: &[u8], batch: &mut ReplyBatch) {
        if batch.refuse_if_full(cid, av::PS3AV_REPLY_VIDEO_GET_HW_CONF_LEN) {
            return;
        }
        batch.reply(false, cid, av::PS3AV_STATUS_SUCCESS, &0u32.to_be_bytes());
    }

    pub(super) fn cid_audio_mode(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
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
}
