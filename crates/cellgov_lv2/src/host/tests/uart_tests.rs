//! Virtual UART dispatch tests: privilege and lifecycle, packet parsing, per-CID replies, HDMI events, and the blocking reader.

use super::*;
use crate::host::test_support::{seed_primary_ppu, FakeRuntime};
use crate::request::Lv2Request;
use cellgov_mem::{GuestAddr, GuestMemory};

const ROOT: u32 = 0x4000_0000;
const PKT_PTR: u32 = 0x1000;
const RX_PTR: u32 = 0x3000;
const PARAMS_PTR: u32 = 0x5000;

fn src() -> UnitId {
    UnitId::new(0)
}

fn root_host() -> Lv2Host {
    let mut host = Lv2Host::new();
    host.set_control_flags1(ROOT);
    host
}

fn runtime_with_packets(bytes: &[u8]) -> FakeRuntime {
    let mut mem = GuestMemory::new(0x10000);
    if !bytes.is_empty() {
        mem.apply_commit(
            ByteRange::new(GuestAddr::new(u64::from(PKT_PTR)), bytes.len() as u64).unwrap(),
            bytes,
        )
        .unwrap();
    }
    FakeRuntime::with_memory(mem)
}

fn pkt(cid: u32, body: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(8 + body.len());
    p.extend_from_slice(&av::PS3AV_VERSION.to_be_bytes());
    p.extend_from_slice(&((body.len() + 4) as u16).to_be_bytes());
    p.extend_from_slice(&cid.to_be_bytes());
    p.extend_from_slice(body);
    p
}

fn code_of(d: &Lv2Dispatch) -> u64 {
    match d {
        Lv2Dispatch::Immediate { code, .. } => *code,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

fn written(d: &Lv2Dispatch) -> Vec<u8> {
    match d {
        Lv2Dispatch::Immediate { code: _, effects } => match &effects[0] {
            Effect::SharedWriteIntent { bytes, .. } => bytes.bytes().to_vec(),
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        },
        other => panic!("expected Immediate with a write, got {other:?}"),
    }
}

fn init(host: &mut Lv2Host, rt: &FakeRuntime) {
    assert_eq!(
        code_of(&host.dispatch(Lv2Request::UartInitialize, src(), rt)),
        0
    );
}

fn send(host: &mut Lv2Host, rt: &FakeRuntime, len: usize, mode: u32) -> Lv2Dispatch {
    host.dispatch(
        Lv2Request::UartSend {
            buf_ptr: PKT_PTR,
            size: len as u64,
            mode,
        },
        src(),
        rt,
    )
}

fn recv(host: &mut Lv2Host, rt: &FakeRuntime, size: u64, mode: u32) -> Lv2Dispatch {
    host.dispatch(
        Lv2Request::UartReceive {
            buf_ptr: RX_PTR,
            size,
            mode,
        },
        src(),
        rt,
    )
}

/// Send `packets` in one buffer and read the whole stream back.
fn roundtrip(host: &mut Lv2Host, packets: &[u8]) -> Vec<u8> {
    let rt = runtime_with_packets(packets);
    init(host, &rt);
    let d = send(host, &rt, packets.len(), av::SYS_UART_MODE_NOT_BLOCKING_OP);
    assert_eq!(code_of(&d), packets.len() as u64);
    let d = recv(host, &rt, 0x800, av::SYS_UART_MODE_NOT_BLOCKING_BIG_OP);
    match d {
        Lv2Dispatch::Immediate { code: 0, .. } => Vec::new(),
        d => written(&d),
    }
}

fn reply_header(reply: &[u8]) -> (u16, u16, u32, u32) {
    (
        rd16(reply, 0),
        rd16(reply, 2),
        rd32(reply, 4),
        rd32(reply, 8),
    )
}

#[test]
fn initialize_needs_root_and_claims_the_uart_once() {
    let rt = runtime_with_packets(&[]);
    let mut host = Lv2Host::new();
    let d = host.dispatch(Lv2Request::UartInitialize, src(), &rt);
    assert_eq!(code_of(&d), u64::from(cell_errors::CELL_ENOSYS));
    assert!(host.state.uart.is_pristine());
    let mut host = root_host();
    init(&mut host, &rt);
    let d = host.dispatch(Lv2Request::UartInitialize, src(), &rt);
    assert_eq!(code_of(&d), u64::from(cell_errors::CELL_EPERM));
}

#[test]
fn send_receive_and_get_params_before_initialize_are_esrch() {
    let p = pkt(av::PS3AV_CID_AV_NULL_CMD, &[0; 4]);
    let rt = runtime_with_packets(&p);
    let mut host = root_host();
    assert_eq!(
        code_of(&send(&mut host, &rt, p.len(), 2)),
        u64::from(cell_errors::CELL_ESRCH)
    );
    assert_eq!(
        code_of(&recv(&mut host, &rt, 16, 0)),
        u64::from(cell_errors::CELL_ESRCH)
    );
    let d = host.dispatch(
        Lv2Request::UartGetParams {
            params_ptr: PARAMS_PTR,
        },
        src(),
        &rt,
    );
    assert_eq!(code_of(&d), u64::from(cell_errors::CELL_ESRCH));
}

#[test]
fn get_params_reports_both_ring_sizes() {
    let rt = runtime_with_packets(&[]);
    let mut host = root_host();
    init(&mut host, &rt);
    let d = host.dispatch(
        Lv2Request::UartGetParams {
            params_ptr: PARAMS_PTR,
        },
        src(),
        &rt,
    );
    let out = written(&d);
    assert_eq!(&out[..8], &0x800u64.to_be_bytes());
    assert_eq!(&out[8..], &0x800u64.to_be_bytes());
}

#[test]
fn av_init_is_acknowledged_with_the_reply_bit_and_records_the_event_mask() {
    let mut host = root_host();
    let stream = roundtrip(
        &mut host,
        &pkt(
            av::PS3AV_CID_AV_INIT,
            &av::PS3AV_EVENT_BIT_PLUGGED.to_be_bytes(),
        ),
    );
    assert_eq!(stream.len(), av::PS3AV_REPLY_HEADER_LEN);
    assert_eq!(
        reply_header(&stream),
        (
            av::PS3AV_VERSION,
            8,
            av::PS3AV_CID_AV_INIT | av::PS3AV_REPLY_BIT,
            av::PS3AV_STATUS_SUCCESS
        )
    );
    assert_eq!(host.state.uart.hdmi_events(), av::PS3AV_EVENT_BIT_PLUGGED);
    assert_eq!(host.obs.uart_cids[&av::PS3AV_CID_AV_INIT], 1);
}

#[test]
fn av_init_with_the_unk_bit_carries_a_four_byte_body() {
    let mut host = root_host();
    let stream = roundtrip(
        &mut host,
        &pkt(
            av::PS3AV_CID_AV_INIT,
            &av::PS3AV_EVENT_BIT_UNK.to_be_bytes(),
        ),
    );
    assert_eq!(stream.len(), av::PS3AV_REPLY_HEADER_LEN + 4);
    assert_eq!(rd16(&stream, 2), 12);
    assert_eq!(&stream[12..], &[0, 0, 0, 0]);
}

#[test]
fn a_blocking_receive_parks_and_the_next_send_wakes_it_with_the_reply() {
    let p = pkt(av::PS3AV_CID_AV_GET_HW_CONF, &[]);
    let rt = runtime_with_packets(&p);
    let mut host = root_host();
    seed_primary_ppu(&mut host, src());
    init(&mut host, &rt);
    let parked = recv(&mut host, &rt, 0x100, av::SYS_UART_MODE_BLOCKING_BIG_OP);
    assert!(
        matches!(
            parked,
            Lv2Dispatch::Block {
                reason: Lv2BlockReason::Uart,
                pending: PendingResponse::ReturnCode { code: 0 },
                ..
            }
        ),
        "got {parked:?}"
    );
    assert_eq!(host.state.uart.readers().len(), 1);
    let second = host.dispatch(
        Lv2Request::UartReceive {
            buf_ptr: RX_PTR + 0x100,
            size: 8,
            mode: av::SYS_UART_MODE_BLOCKING_BIG_OP,
        },
        UnitId::new(1),
        &rt,
    );
    assert_eq!(
        code_of(&second),
        u64::from(cell_errors::CELL_ESRCH),
        "no thread record"
    );
    let d = send(&mut host, &rt, p.len(), av::SYS_UART_MODE_NOT_BLOCKING_OP);
    match d {
        Lv2Dispatch::WakeAndReturn {
            code,
            woken_unit_ids,
            response_updates,
            effects,
        } => {
            assert_eq!(code, p.len() as u64);
            assert_eq!(woken_unit_ids, vec![src()]);
            assert_eq!(
                response_updates,
                vec![(src(), PendingResponse::ReturnCode { code: 20 })]
            );
            let Effect::SharedWriteIntent { bytes, .. } = &effects[0] else {
                panic!("expected a write");
            };
            let reply = bytes.bytes();
            assert_eq!(reply.len(), 20);
            assert_eq!(
                reply_header(reply),
                (
                    av::PS3AV_VERSION,
                    16,
                    av::PS3AV_CID_AV_GET_HW_CONF | av::PS3AV_REPLY_BIT,
                    av::PS3AV_STATUS_SUCCESS
                )
            );
            assert_eq!(&reply[12..], &[0, 1, 0, 1, 0, 1, 0, 1]);
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    assert!(host.state.uart.readers().is_empty());
    assert!(host.state.uart.pending_bytes().is_empty());
}

#[test]
fn receive_on_an_empty_stream_returns_zero_without_blocking() {
    let rt = runtime_with_packets(&[]);
    let mut host = root_host();
    init(&mut host, &rt);
    assert_eq!(
        code_of(&recv(
            &mut host,
            &rt,
            64,
            av::SYS_UART_MODE_NOT_BLOCKING_BIG_OP
        )),
        0
    );
    assert_eq!(
        code_of(&recv(&mut host, &rt, 64, 2)),
        u64::from(cell_errors::CELL_EINVAL)
    );
}

#[test]
fn syscon_replies_follow_plain_replies_and_hdmi_0_monitor_info_is_204_bytes() {
    let mut packets = pkt(av::PS3AV_CID_AV_GET_MONITOR_INFO, &[0, 0, 0, 0]);
    packets.extend(pkt(av::PS3AV_CID_AV_NULL_CMD, &[0; 4]));
    let mut host = root_host();
    let stream = roundtrip(&mut host, &packets);
    assert_eq!(stream.len(), 12 + 12 + av::PS3AV_MONITOR_INFO_HDMI_LEN);
    assert_eq!(
        rd32(&stream, 4),
        av::PS3AV_CID_AV_NULL_CMD | av::PS3AV_REPLY_BIT
    );
    let monitor = &stream[12..];
    assert_eq!(
        reply_header(monitor),
        (
            av::PS3AV_VERSION,
            (av::PS3AV_MONITOR_INFO_HDMI_LEN + 8) as u16,
            av::PS3AV_CID_AV_GET_MONITOR_INFO | av::PS3AV_REPLY_BIT,
            av::PS3AV_STATUS_SUCCESS
        )
    );
    let info = &monitor[12..];
    assert_eq!(info[11], av::PS3AV_MONITOR_TYPE_HDMI);
    assert_eq!(&info[12..24], b"CellGov HDMI");
    assert_eq!(&info[24..28], &[0u8; 4], "the name field is NUL-padded");
    assert_eq!(rd32(info, 32), av::PS3AV_RESBIT_1920X1080P, "native 60 Hz");
    assert_eq!(rd32(info, 40), 0, "the 50 Hz table has no native timing");
    assert_eq!(rd32(info, 52), 0, "an HDTV carries no VESA mode");
    assert_eq!(rd16(info, 86), 7, "audio blocks");
}

#[test]
fn the_hdmi_monitor_advertises_only_cea_modes_and_its_native_timing_is_among_them() {
    let mut host = root_host();
    let stream = roundtrip(
        &mut host,
        &pkt(av::PS3AV_CID_AV_GET_MONITOR_INFO, &[0, 0, 0, 0]),
    );
    assert_eq!(stream.len(), 12 + av::PS3AV_MONITOR_INFO_HDMI_LEN);
    let info = &stream[12..];
    let hd = av::PS3AV_RESBIT_1280X720P | av::PS3AV_RESBIT_1920X1080I | av::PS3AV_RESBIT_1920X1080P;
    let res_60 = rd32(info, 28);
    let res_50 = rd32(info, 36);
    assert_eq!(res_60, av::PS3AV_RESBIT_720X480P | hd);
    assert_eq!(res_50, av::PS3AV_RESBIT_720X576P | hd);
    assert_eq!(
        res_60 & rd32(info, 32),
        rd32(info, 32),
        "the native timing must be one of the modes the same table advertises"
    );
    assert_eq!(rd32(info, 44), 0, "no res_other table");
    assert_eq!(rd32(info, 52), 0, "no VESA table");
    assert_eq!(
        &info[160..200],
        &[0u8; 40],
        "the five 3D resolution blocks stay empty"
    );
}

#[test]
fn monitor_info_on_hdmi_1_fails_syscon_and_avmulti_gets_the_full_fixture() {
    let mut host = root_host();
    let stream = roundtrip(
        &mut host,
        &pkt(av::PS3AV_CID_AV_GET_MONITOR_INFO, &[0, 1, 0, 0]),
    );
    assert_eq!(stream.len(), 12);
    assert_eq!(rd32(&stream, 8), av::PS3AV_STATUS_SYSCON_COMMUNICATE_FAIL);
    let mut host = root_host();
    let stream = roundtrip(
        &mut host,
        &pkt(av::PS3AV_CID_AV_GET_MONITOR_INFO, &[0, 0x10, 0, 0]),
    );
    assert_eq!(stream.len(), 12 + av::PS3AV_MONITOR_INFO_LEN);
    assert_eq!(stream[12], 0x10);
    assert_eq!(stream[12 + 11], av::PS3AV_MONITOR_TYPE_AVMULTI);
    let mut host = root_host();
    let stream = roundtrip(
        &mut host,
        &pkt(av::PS3AV_CID_AV_GET_MONITOR_INFO, &[0, 0x20, 0, 0]),
    );
    assert_eq!(rd32(&stream, 8), av::PS3AV_STATUS_INVALID_PORT);
}

#[test]
fn bksv_list_and_aksv_report_the_fixed_keys() {
    let mut host = root_host();
    let stream = roundtrip(
        &mut host,
        &pkt(av::PS3AV_CID_AV_GET_BKSV_LIST, &[0, 0, 0, 0]),
    );
    assert_eq!(stream.len(), 12 + 16);
    assert_eq!(rd32(&stream, 16), 1, "ksv_cnt");
    assert_eq!(&stream[20..25], &av::PS3AV_BKSV_VALUE);
    let mut host = root_host();
    let stream = roundtrip(&mut host, &pkt(av::PS3AV_CID_AV_GET_AKSV, &[]));
    assert_eq!(stream.len(), 12 + av::PS3AV_REPLY_GET_AKSV_LEN);
    assert_eq!(rd32(&stream, 12), 5);
    assert_eq!(&stream[16..21], &av::PS3AV_AKSV_VALUE);
}

#[test]
fn an_unknown_cid_gets_no_reply_and_is_witnessed() {
    let mut host = root_host();
    let stream = roundtrip(&mut host, &pkt(av::PS3AV_CID_AV_GET_PORT_STATE, &[0; 4]));
    assert!(stream.is_empty());
    assert_eq!(
        host.obs.uart_unknown_cids[&av::PS3AV_CID_AV_GET_PORT_STATE],
        1
    );
    assert_eq!(
        host.obs.invariant_break_sites["dispatch.uart_unknown_cid"],
        1
    );
}

#[test]
fn a_wrong_version_answers_invalid_command_and_stops_the_batch() {
    let mut packets = pkt(av::PS3AV_CID_AV_NULL_CMD, &[0; 4]);
    packets[0] = 0x02;
    packets[1] = 0x04;
    packets.extend(pkt(av::PS3AV_CID_AV_NULL_CMD, &[0; 4]));
    let mut host = root_host();
    let stream = roundtrip(&mut host, &packets);
    assert_eq!(stream.len(), 12, "one reply, second packet never parsed");
    assert_eq!(
        rd32(&stream, 4),
        av::PS3AV_CID_AV_NULL_CMD | av::PS3AV_REPLY_BIT
    );
    assert_eq!(rd32(&stream, 8), av::PS3AV_STATUS_INVALID_COMMAND);
}

#[test]
fn a_size_mismatch_answers_invalid_sample_size() {
    let mut host = root_host();
    let stream = roundtrip(&mut host, &pkt(av::PS3AV_CID_AV_ENABLE_EVENT, &[0; 8]));
    assert_eq!(rd32(&stream, 8), av::PS3AV_STATUS_INVALID_SAMPLE_SIZE);
    assert_eq!(host.state.uart.hdmi_events(), 0);
}

#[test]
fn hdmi_mode_stages_unplugged_then_plugged_only_for_enabled_bits() {
    let mut packets = pkt(
        av::PS3AV_CID_AV_INIT,
        &(av::PS3AV_EVENT_BIT_UNPLUGGED | av::PS3AV_EVENT_BIT_PLUGGED).to_be_bytes(),
    );
    packets.extend(pkt(av::PS3AV_CID_AV_HDMI_MODE, &[0xFF, 0, 0, 0]));
    let mut host = root_host();
    let stream = roundtrip(&mut host, &packets);
    // AV_INIT reply (plain), HDMI_MODE reply (syscon), then the events.
    assert_eq!(
        stream.len(),
        12 + 12 + 8 + 8 + av::PS3AV_MONITOR_INFO_HDMI_LEN
    );
    assert_eq!(
        rd32(&stream, 16),
        av::PS3AV_CID_AV_HDMI_MODE | av::PS3AV_REPLY_BIT
    );
    let unplugged = &stream[24..32];
    assert_eq!(
        rd16(unplugged, 0),
        av::PS3AV_VERSION,
        "events echo the AV_INIT version"
    );
    assert_eq!(rd16(unplugged, 2), 4);
    assert_eq!(rd32(unplugged, 4), av::PS3AV_CID_EVENT_UNPLUGGED);
    let plugged = &stream[32..];
    assert_eq!(rd16(plugged, 2), av::PS3AV_MONITOR_INFO_LEN as u16);
    assert_eq!(rd32(plugged, 4), av::PS3AV_CID_EVENT_PLUGGED);
    assert_eq!(plugged[8 + 11], av::PS3AV_MONITOR_TYPE_HDMI);
    assert_eq!(host.obs.uart_events_gated, 0);

    let mut host = root_host();
    let stream = roundtrip(
        &mut host,
        &pkt(av::PS3AV_CID_AV_HDMI_MODE, &[0xFF, 0, 0, 0]),
    );
    assert_eq!(stream.len(), 12, "nothing enabled: reply only");
    assert_eq!(host.obs.uart_events_gated, 2);
}

#[test]
fn hdmi_mode_with_hdcp_off_is_unsupported_and_changes_nothing() {
    let mut host = root_host();
    let stream = roundtrip(
        &mut host,
        &pkt(
            av::PS3AV_CID_AV_HDMI_MODE,
            &[av::PS3AV_HDMI_BEHAVIOR_HDCP_OFF, 0, 0, 0],
        ),
    );
    assert_eq!(rd32(&stream, 8), av::PS3AV_STATUS_UNSUPPORTED_HDMI_MODE);
}

fn video_mode_packet(head: u32, vid: u32, width: u32, height: u32) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&head.to_be_bytes());
    body.extend_from_slice(&[0, 0, 0, 0]); // unk1, unk2
    body.extend_from_slice(&vid.to_be_bytes());
    body.extend_from_slice(&width.to_be_bytes());
    body.extend_from_slice(&height.to_be_bytes());
    body.extend_from_slice(&(width * 4).to_be_bytes()); // pitch
    body.extend_from_slice(&0u32.to_be_bytes()); // video_out_format
    body.extend_from_slice(&0u32.to_be_bytes()); // video_format
    body.extend_from_slice(&[0, 0, 0, 0]); // unk3, video_order
    body.extend_from_slice(&0u32.to_be_bytes()); // unk4
    pkt(av::PS3AV_CID_VIDEO_MODE, &body)
}

fn av_video_packet(avport: u16, av_vid: u16) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&avport.to_be_bytes());
    body.extend_from_slice(&av_vid.to_be_bytes());
    body.extend_from_slice(&[0; 12]);
    pkt(0, &body)
}

fn avb_param(video: &[Vec<u8>], av_video: &[Vec<u8>], av_audio: &[Vec<u8>]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&(video.len() as u16).to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    body.extend_from_slice(&(av_video.len() as u16).to_be_bytes());
    body.extend_from_slice(&(av_audio.len() as u16).to_be_bytes());
    for p in video.iter().chain(av_video).chain(av_audio) {
        body.extend_from_slice(p);
    }
    pkt(av::PS3AV_CID_AVB_PARAM, &body)
}

#[test]
fn avb_param_video_only_reports_the_mode_verdict() {
    let mut host = root_host();
    let ok = avb_param(&[video_mode_packet(0, 10, 1280, 720)], &[], &[]);
    let stream = roundtrip(&mut host, &ok);
    assert_eq!(rd32(&stream, 8), av::PS3AV_STATUS_SUCCESS);
    let mut host = root_host();
    let bad = avb_param(&[video_mode_packet(1, 10, 1920, 720)], &[], &[]);
    let stream = roundtrip(&mut host, &bad);
    assert_eq!(rd32(&stream, 8), av::PS3AV_STATUS_INVALID_VIDEO_PARAM);
    assert!(host.state.uart.head_b_initialized);
}

#[test]
fn avb_param_with_an_hdmi_av_video_packet_answers_via_syscon_and_fires_hdcp_done() {
    let mut packets = pkt(
        av::PS3AV_CID_AV_INIT,
        &av::PS3AV_EVENT_BIT_HDCP_DONE.to_be_bytes(),
    );
    packets.extend(avb_param(
        &[video_mode_packet(0, 10, 1280, 720)],
        &[av_video_packet(0, 3)],
        &[],
    ));
    let mut host = root_host();
    let stream = roundtrip(&mut host, &packets);
    // AV_INIT reply, AVB_PARAM reply (syscon), HDCP_DONE event (20 bytes).
    assert_eq!(stream.len(), 12 + 12 + 20);
    assert_eq!(
        rd32(&stream, 16),
        av::PS3AV_CID_AVB_PARAM | av::PS3AV_REPLY_BIT
    );
    assert_eq!(rd32(&stream, 20), av::PS3AV_STATUS_SUCCESS);
    let event = &stream[24..];
    assert_eq!(rd16(event, 2), 16);
    assert_eq!(rd32(event, 4), av::PS3AV_CID_EVENT_HDCP_DONE);
    assert_eq!(rd32(event, 8), 1, "ksv_cnt");
    assert_eq!(&event[12..17], &av::PS3AV_BKSV_VALUE);
    assert!(host.state.uart.hdmi_res_set[0]);
    // A second authentication reports a re-auth only when that bit is on.
    let rt = runtime_with_packets(&avb_param(&[], &[av_video_packet(0, 3)], &[]));
    let n = avb_param(&[], &[av_video_packet(0, 3)], &[]).len();
    send(&mut host, &rt, n, av::SYS_UART_MODE_NOT_BLOCKING_OP);
    let d = recv(&mut host, &rt, 0x800, av::SYS_UART_MODE_NOT_BLOCKING_BIG_OP);
    assert_eq!(written(&d).len(), 12, "reply only; reauth bit off");
    assert_eq!(host.obs.uart_events_gated, 1);
}

#[test]
fn avb_param_av_video_on_hdmi_1_fails_syscon_and_a_bad_vid_is_invalid() {
    let mut host = root_host();
    let stream = roundtrip(&mut host, &avb_param(&[], &[av_video_packet(1, 3)], &[]));
    assert_eq!(rd32(&stream, 8), av::PS3AV_STATUS_SYSCON_COMMUNICATE_FAIL);
    let mut host = root_host();
    let stream = roundtrip(&mut host, &avb_param(&[], &[av_video_packet(0, 24)], &[]));
    assert_eq!(rd32(&stream, 8), av::PS3AV_STATUS_INVALID_AV_PARAM);
}

#[test]
fn avb_param_with_too_many_sections_is_a_size_mismatch() {
    let mut body = vec![0u8; 8];
    body[0..2].copy_from_slice(&3u16.to_be_bytes());
    let mut host = root_host();
    let stream = roundtrip(&mut host, &pkt(av::PS3AV_CID_AVB_PARAM, &body));
    assert_eq!(rd32(&stream, 8), av::PS3AV_STATUS_INVALID_SAMPLE_SIZE);
}

#[test]
fn an_avb_param_shorter_than_its_own_counts_is_a_size_mismatch_not_a_fault() {
    for body_len in [0usize, 4] {
        let mut host = root_host();
        let stream = roundtrip(
            &mut host,
            &pkt(av::PS3AV_CID_AVB_PARAM, &vec![0u8; body_len]),
        );
        assert_eq!(
            rd32(&stream, 8),
            av::PS3AV_STATUS_INVALID_SAMPLE_SIZE,
            "{body_len}-byte body"
        );
    }
}

#[test]
fn the_walk_advances_by_the_declared_length_even_below_a_header() {
    // AUDIO_MUTE has no size rule: with length 0 it is acknowledged
    // and the walk resumes four bytes in, inside its own cid, where
    // the version check fails and stops the batch before the second
    // packet.
    let mut packets = pkt(av::PS3AV_CID_AUDIO_MUTE, &[]);
    packets[2] = 0;
    packets[3] = 0;
    packets.extend(pkt(av::PS3AV_CID_AV_NULL_CMD, &[0; 4]));
    let mut host = root_host();
    let stream = roundtrip(&mut host, &packets);
    assert_eq!(stream.len(), 24);
    assert_eq!(
        rd32(&stream, 4),
        av::PS3AV_CID_AUDIO_MUTE | av::PS3AV_REPLY_BIT
    );
    assert_eq!(rd32(&stream, 8), av::PS3AV_STATUS_SUCCESS);
    // At offset 4 the header straddles the first packet's cid and the
    // second packet's start: version 0x0200, length 3, and a cid word
    // of 0x02050008 whose low half addresses the refusal.
    assert_eq!(rd32(&stream, 16), 0x0008 | av::PS3AV_REPLY_BIT);
    assert_eq!(rd32(&stream, 20), av::PS3AV_STATUS_INVALID_COMMAND);
    assert!(!host.obs.uart_cids.contains_key(&av::PS3AV_CID_AV_NULL_CMD));
}

#[test]
fn audio_mode_rejects_an_unknown_port_or_rate() {
    let mut body = vec![0u8; 60];
    body[0] = 0x11; // AVMULTI_1
    let mut host = root_host();
    let stream = roundtrip(&mut host, &pkt(av::PS3AV_CID_AUDIO_MODE, &body));
    assert_eq!(rd32(&stream, 8), av::PS3AV_STATUS_INVALID_AUDIO_PARAM);
    let mut body = vec![0u8; 60];
    body[12..16].copy_from_slice(&3u32.to_be_bytes()); // 48 kHz
    let mut host = root_host();
    let stream = roundtrip(&mut host, &pkt(av::PS3AV_CID_AUDIO_MODE, &body));
    assert_eq!(rd32(&stream, 8), av::PS3AV_STATUS_SUCCESS);
}

#[test]
fn video_route_is_never_selectable_and_video_pitch_is_validated() {
    let mut host = root_host();
    let stream = roundtrip(&mut host, &pkt(av::PS3AV_CID_VIDEO_ROUTE, &[0; 16]));
    assert_eq!(rd32(&stream, 8), av::PS3AV_STATUS_NO_SEL);
    let mut body = Vec::new();
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&5124u32.to_be_bytes()); // not 8-aligned
    let mut host = root_host();
    let stream = roundtrip(&mut host, &pkt(av::PS3AV_CID_VIDEO_PITCH, &body));
    assert_eq!(rd32(&stream, 8), av::PS3AV_STATUS_INVALID_VIDEO_PARAM);
}

#[test]
fn a_send_over_the_tx_ring_is_eagain_whole_or_refuse_and_overflow_otherwise() {
    // One byte over the ring: whole-or-refuse refuses it, the blocking
    // mode accepts it and the AV manager answers a single overflow.
    let big = vec![0u8; 0x801];
    let rt = runtime_with_packets(&big);
    let mut host = root_host();
    init(&mut host, &rt);
    assert_eq!(
        code_of(&send(
            &mut host,
            &rt,
            big.len(),
            av::SYS_UART_MODE_NOT_BLOCKING_OP
        )),
        u64::from(cell_errors::CELL_EAGAIN)
    );
    assert_eq!(
        code_of(&send(
            &mut host,
            &rt,
            big.len(),
            av::SYS_UART_MODE_BLOCKING_BIG_OP
        )),
        0x801
    );
    let d = recv(&mut host, &rt, 0x800, av::SYS_UART_MODE_NOT_BLOCKING_BIG_OP);
    let stream = written(&d);
    assert_eq!(stream.len(), 12);
    assert_eq!(rd32(&stream, 8), av::PS3AV_STATUS_BUFFER_OVERFLOW);
}

#[test]
fn a_chunked_non_blocking_send_over_the_ring_reports_its_first_chunk() {
    let big = vec![0u8; 0x1001];
    let rt = runtime_with_packets(&big);
    let mut host = root_host();
    init(&mut host, &rt);
    assert_eq!(
        code_of(&send(
            &mut host,
            &rt,
            big.len(),
            av::SYS_UART_MODE_NOT_BLOCKING_BIG_OP
        )),
        av::SYS_UART_CHUNK,
        "one 4 KiB chunk, not the whole buffer"
    );
    let d = recv(&mut host, &rt, 0x800, av::SYS_UART_MODE_NOT_BLOCKING_BIG_OP);
    let stream = written(&d);
    assert_eq!(stream.len(), 12);
    assert_eq!(rd32(&stream, 8), av::PS3AV_STATUS_BUFFER_OVERFLOW);
    // A buffer within one chunk is reported whole even past the ring.
    assert_eq!(
        code_of(&send(
            &mut host,
            &rt,
            0x801,
            av::SYS_UART_MODE_NOT_BLOCKING_BIG_OP
        )),
        0x801
    );
}

#[test]
fn a_transfer_over_the_cap_is_einval_with_a_named_break() {
    let rt = runtime_with_packets(&[]);
    let mut host = root_host();
    init(&mut host, &rt);
    let d = recv(&mut host, &rt, av::SYS_UART_MAX_TRANSFER + 1, 0);
    assert_eq!(code_of(&d), u64::from(cell_errors::CELL_EINVAL));
    assert_eq!(
        host.obs.invariant_break_sites["dispatch.uart_transfer_over_cap"],
        1
    );
}

#[test]
fn the_cid_table_names_each_command_once() {
    let mut seen = std::collections::BTreeSet::new();
    for spec in CID_TABLE {
        assert!(seen.insert(spec.cid), "cid 0x{:08x} listed twice", spec.cid);
    }
    assert_eq!(seen.len(), CID_TABLE.len());
    for cid in [
        av::PS3AV_CID_AV_INIT,
        av::PS3AV_CID_AVB_PARAM,
        av::PS3AV_CID_AUDIO_MUTE,
    ] {
        assert!(cid_spec(cid).is_some(), "0x{cid:08x} missing");
    }
    assert!(cid_spec(av::PS3AV_CID_AV_GET_PORT_STATE).is_none());
}

#[test]
fn every_cid_table_row_answers_a_packet_of_its_declared_size() {
    for spec in CID_TABLE {
        let body_len = match spec.size {
            SizeRule::Exact(n) => n - av::PS3AV_HEADER_LEN,
            // An unchecked command still carries a header; a computed
            // one with zero sub-packet counts is its bare header.
            SizeRule::Unchecked => 4,
            SizeRule::Computed(_) => av::PS3AV_PKT_INC_AVSET_LEN - av::PS3AV_HEADER_LEN,
        };
        let packet = pkt(spec.cid, &vec![0u8; body_len]);
        let mut host = root_host();
        let stream = roundtrip(&mut host, &packet);
        assert!(
            stream.len() >= av::PS3AV_REPLY_HEADER_LEN,
            "cid 0x{:08x}: no reply to a {}-byte packet",
            spec.cid,
            packet.len()
        );
        assert_eq!(
            rd32(&stream, 4),
            spec.cid | av::PS3AV_REPLY_BIT,
            "cid 0x{:08x}: reply addressed to another cid",
            spec.cid
        );
        assert_ne!(
            rd32(&stream, 8),
            av::PS3AV_STATUS_INVALID_SAMPLE_SIZE,
            "cid 0x{:08x}: size rule refuses its own size",
            spec.cid
        );
        assert!(
            !host
                .obs
                .invariant_break_sites
                .contains_key("dispatch.uart_unknown_cid"),
            "cid 0x{:08x}: parsed but unknown",
            spec.cid
        );
        assert!(host.obs.uart_unknown_cids.is_empty());
    }
}

#[test]
fn uart_state_folds_into_the_host_hash_only_once_initialized() {
    let rt = runtime_with_packets(&[]);
    let mut host = root_host();
    let before = host.state_hash();
    let mut twin = host.clone();
    init(&mut host, &rt);
    assert_ne!(host.state_hash(), before);
    init(&mut twin, &rt);
    assert_eq!(host.state_hash(), twin.state_hash());
}
