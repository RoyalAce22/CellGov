//! AVB_PARAM and the HDMI plug and HDCP event script.

use cellgov_ps3_abi::lv2::uart as av;

use crate::host::Lv2Host;

use super::av_data::{hdmi_monitor_info, ksv_list_body, video_mode_status};
use super::packet::{padded, rd16, rd32};
use super::reply::ReplyBatch;

/// HDMI link states the event script walks, in order; 0 is the
/// "before any state" floor the script can start from.
pub(super) const HDMI_STATE_UNPLUGGED: u8 = 1;
pub(super) const HDMI_STATE_PLUGGED: u8 = 2;
pub(super) const HDMI_STATE_HDCP_DONE: u8 = 3;

impl Lv2Host {
    /// AVB_PARAM: a video-mode section, an AV-video section, and an
    /// AV-audio section, each validated in turn.
    pub(super) fn uart_inc_avset(&mut self, cid: u32, pkt: &[u8], batch: &mut ReplyBatch) {
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
    pub(super) fn uart_hdmi_script(&mut self, first: u8, last: u8, batch: &mut ReplyBatch) {
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
