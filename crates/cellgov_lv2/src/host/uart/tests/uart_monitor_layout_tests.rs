//! Field placement in the two synthesised `ps3av_monitor_info`
//! descriptors, pinned at the byte level against the offset table.

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::FakeRuntime;
use crate::host::Lv2Host;
use crate::request::Lv2Request;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_mem::{GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::uart as av;
use cellgov_ps3_abi::lv2::uart::monitor_info as mi;

const ROOT: u32 = 0x4000_0000;
const PKT_PTR: u32 = 0x1000;
const RX_PTR: u32 = 0x3000;
/// Header ahead of the descriptor in a GET_MONITOR_INFO reply.
const REPLY_HEAD: usize = 12;

fn src() -> UnitId {
    UnitId::new(0)
}

fn root_host() -> Lv2Host {
    let mut host = Lv2Host::new();
    host.set_control_flags1(ROOT);
    host
}

fn pkt(cid: u32, body: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(8 + body.len());
    p.extend_from_slice(&av::PS3AV_VERSION.to_be_bytes());
    p.extend_from_slice(&((body.len() + 4) as u16).to_be_bytes());
    p.extend_from_slice(&cid.to_be_bytes());
    p.extend_from_slice(body);
    p
}

/// The descriptor a GET_MONITOR_INFO on `avport` replies with.
fn descriptor(avport: u8) -> Vec<u8> {
    let packets = pkt(av::PS3AV_CID_AV_GET_MONITOR_INFO, &[0, avport, 0, 0]);
    let mut mem = GuestMemory::new(0x10000);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(u64::from(PKT_PTR)), packets.len() as u64).unwrap(),
        &packets,
    )
    .unwrap();
    let rt = FakeRuntime::with_memory(mem);
    let mut host = root_host();
    assert_eq!(
        match host.dispatch(Lv2Request::UartInitialize, src(), &rt) {
            Lv2Dispatch::Immediate { code, .. } => code,
            other => panic!("expected Immediate, got {other:?}"),
        },
        0
    );
    host.dispatch(
        Lv2Request::UartSend {
            buf_ptr: PKT_PTR,
            size: packets.len() as u64,
            mode: av::SYS_UART_MODE_NOT_BLOCKING_OP,
        },
        src(),
        &rt,
    );
    let d = host.dispatch(
        Lv2Request::UartReceive {
            buf_ptr: RX_PTR,
            size: 0x800,
            mode: av::SYS_UART_MODE_NOT_BLOCKING_BIG_OP,
        },
        src(),
        &rt,
    );
    let stream = match &d {
        Lv2Dispatch::Immediate { effects, .. } => match &effects[0] {
            Effect::SharedWriteIntent { bytes, .. } => bytes.bytes().to_vec(),
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        },
        other => panic!("expected Immediate with a write, got {other:?}"),
    };
    stream[REPLY_HEAD..].to_vec()
}

fn res_bits(m: &[u8], table: usize) -> u32 {
    let at = mi::RES_TABLE_OFFSET + table * mi::RES_TABLE_SIZE;
    u32::from_be_bytes(m[at..at + mi::RES_WORD_SIZE].try_into().unwrap())
}

fn res_native(m: &[u8], table: usize) -> u32 {
    let at = mi::RES_TABLE_OFFSET + table * mi::RES_TABLE_SIZE + mi::RES_WORD_SIZE;
    u32::from_be_bytes(m[at..at + mi::RES_WORD_SIZE].try_into().unwrap())
}

#[test]
fn avmulti_fills_the_60_50_and_vesa_tables_and_leaves_res_other_zero() {
    let m = descriptor(av::PS3AV_AVPORT_AVMULTI_0 as u8);
    assert_eq!(res_bits(&m, 0), u32::MAX, "res_60 at 28");
    assert_eq!(res_bits(&m, 1), u32::MAX, "res_50 at 36");
    assert_eq!(res_bits(&m, 2), 0, "res_other at 44 is untouched");
    assert_eq!(res_bits(&m, 3), u32::MAX, "res_vesa at 52");
    // `avmulti_monitor_info` fills only the bit words, so no table
    // names a native timing.
    for table in 0..mi::RES_TABLE_COUNT {
        assert_eq!(res_native(&m, table), 0, "table {table} native word");
    }
}

#[test]
fn avmulti_carries_one_stereo_lpcm_block_and_no_screen_size() {
    let m = descriptor(av::PS3AV_AVPORT_AVMULTI_0 as u8);
    assert_eq!(m[mi::AVPORT_OFFSET], av::PS3AV_AVPORT_AVMULTI_0 as u8);
    assert_eq!(m[mi::MONITOR_TYPE_OFFSET], av::PS3AV_MONITOR_TYPE_AVMULTI);
    assert_eq!(m[mi::SPEAKER_INFO_OFFSET], 1);
    assert_eq!(
        u16::from_be_bytes(
            m[mi::NUM_AUDIO_BLOCK_OFFSET..mi::NUM_AUDIO_BLOCK_OFFSET + 2]
                .try_into()
                .unwrap()
        ),
        1
    );
    assert_eq!(
        &m[mi::AUDIO_BLOCK_OFFSET..mi::AUDIO_BLOCK_OFFSET + mi::AUDIO_BLOCK_SIZE],
        &[av::PS3AV_MON_INFO_AUDIO_TYPE_LPCM, 2, 127, 7]
    );
    // An analogue port reports no panel dimensions.
    let hor = &m[mi::HOR_SCREEN_SIZE_OFFSET..mi::HOR_SCREEN_SIZE_OFFSET + 2];
    let ver = &m[mi::VER_SCREEN_SIZE_OFFSET..mi::VER_SCREEN_SIZE_OFFSET + 2];
    assert_eq!(u16::from_be_bytes(hor.try_into().unwrap()), 0);
    assert_eq!(u16::from_be_bytes(ver.try_into().unwrap()), 0);
}

#[test]
fn hdmi_pairs_each_resolution_table_with_its_native_word() {
    let m = descriptor(av::PS3AV_AVPORT_HDMI_0 as u8);
    let hd = av::PS3AV_RESBIT_1280X720P | av::PS3AV_RESBIT_1920X1080I | av::PS3AV_RESBIT_1920X1080P;
    assert_eq!(res_bits(&m, 0), av::PS3AV_RESBIT_720X480P | hd);
    assert_eq!(res_bits(&m, 1), av::PS3AV_RESBIT_720X576P | hd);
    assert_eq!(res_bits(&m, 2), 0, "res_other");
    assert_eq!(res_bits(&m, 3), 0, "a TV carries no VESA mode");
    // The native timing is 60 Hz, so only the 60 Hz table names one.
    assert_eq!(res_native(&m, 0), av::PS3AV_RESBIT_1920X1080P);
    assert_eq!(res_native(&m, 1), 0);
}

#[test]
fn hdmi_audio_blocks_stop_before_the_screen_size_field() {
    let m = descriptor(av::PS3AV_AVPORT_HDMI_0 as u8);
    let count = u16::from_be_bytes(
        m[mi::NUM_AUDIO_BLOCK_OFFSET..mi::NUM_AUDIO_BLOCK_OFFSET + 2]
            .try_into()
            .unwrap(),
    );
    assert_eq!(count, 7);
    let end = mi::AUDIO_BLOCK_OFFSET + usize::from(count) * mi::AUDIO_BLOCK_SIZE;
    assert!(
        end <= mi::HOR_SCREEN_SIZE_OFFSET,
        "audio blocks end at {end}, past hor_screen_size"
    );
    assert_eq!(
        m[mi::AUDIO_BLOCK_OFFSET],
        av::PS3AV_MON_INFO_AUDIO_TYPE_LPCM
    );
    assert_eq!(
        m[end - mi::AUDIO_BLOCK_SIZE],
        av::PS3AV_MON_INFO_AUDIO_TYPE_DOLBY_THD
    );
    assert!(m[end..mi::HOR_SCREEN_SIZE_OFFSET].iter().all(|&b| b == 0));
}

#[test]
fn hdmi_writes_all_eight_chromaticity_words_then_gamma() {
    let m = descriptor(av::PS3AV_AVPORT_HDMI_0 as u8);
    let coords: Vec<u16> = (0..mi::COLOR_COORD_COUNT)
        .map(|i| {
            let at = mi::COLOR_COORD_OFFSET + i * mi::COLOR_COORD_SIZE;
            u16::from_be_bytes(m[at..at + mi::COLOR_COORD_SIZE].try_into().unwrap())
        })
        .collect();
    // Rec.709 primaries and a D65 white point, x then y per colour.
    assert_eq!(coords, vec![655, 338, 307, 614, 154, 61, 320, 337]);
    assert_eq!(
        u32::from_be_bytes(
            m[mi::GAMMA_OFFSET..mi::GAMMA_OFFSET + 4]
                .try_into()
                .unwrap()
        ),
        100,
        "gamma follows the last coordinate word"
    );
}
