//! The video-mode bounds and the fixed monitor descriptors the AV manager reports.

use cellgov_ps3_abi::lv2::uart as av;

use super::packet::{put_bytes, rd16, rd32};

/// Video-mode bounds table indexed by the vid map below:
/// `(width_div, width, height)`.
///
/// No dev_flash module carries this table; the ladder belongs to the
/// system controller. Its contents and its order are CellGov's own,
/// and both are unestablished against hardware.
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
/// The bounds are [`cellgov_ps3_abi::lv2::uart::video_mode`]. None of
/// them has a witness, so a rejection here is a guess.
pub(super) fn video_mode_status(v: &[u8]) -> u32 {
    use cellgov_ps3_abi::lv2::uart::video_mode as vm;
    let head = rd32(v, vm::HEAD_OFFSET);
    let unk2 = rd16(v, vm::UNK2_OFFSET);
    let vid = rd32(v, vm::VID_OFFSET);
    let width = rd32(v, vm::WIDTH_OFFSET);
    let height = rd32(v, vm::HEIGHT_OFFSET);
    let pitch = rd32(v, vm::PITCH_OFFSET);
    let out_format = rd32(v, vm::OUT_FORMAT_OFFSET);
    let format = rd32(v, vm::FORMAT_OFFSET);
    let order = rd16(v, vm::ORDER_OFFSET);
    let Some(idx) = video_sce_index(vid) else {
        return av::PS3AV_STATUS_INVALID_VIDEO_PARAM;
    };
    let (width_div, max_width, max_height) = VIDEO_SCE_PARAMS[idx];
    let bad = head > av::PS3AV_HEAD_B_ANALOG
        || order > vm::ORDER_MAX
        || format > vm::FORMAT_MAX
        || out_format > vm::FORMAT_MAX
        || (1u64 << out_format) & vm::OUT_FORMAT_ACCEPTED == 0
        || unk2 > vm::UNK2_MAX
        || pitch & vm::ALIGN_MASK != 0
        || pitch > u32::from(u16::MAX)
        || (width != vm::WIDTH_UNALIGNED_EXEMPT
            && (width & vm::ALIGN_MASK != 0 || width > u32::from(u16::MAX)))
        || (max_width != vm::WIDTH_UNCHECKED_MAX && width > max_width / width_div)
        || !((height == vm::HEIGHT_TALL && vm::HEIGHT_TALL_MAX_HEIGHTS.contains(&max_height))
            || (height <= max_height && height <= u32::from(u16::MAX)));
    if bad {
        av::PS3AV_STATUS_INVALID_VIDEO_PARAM
    } else {
        av::PS3AV_STATUS_SUCCESS
    }
}

/// `monitor_name`, NUL-padded into the descriptor's name field.
const MONITOR_NAME: &[u8] = b"CellGov HDMI";
const _: () = assert!(MONITOR_NAME.len() <= av::monitor_info::MONITOR_NAME_LEN);

/// `monitor_id` for the synthesised HDMI sink.
///
/// On a console this is the attached display's EDID identification
/// block. CellGov has no display, so the bytes are its own pick.
/// Nothing here holds them against the EDID vendor registry.
const MONITOR_ID: [u8; av::monitor_info::MONITOR_ID_LEN] =
    [0x4A, 0x13, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x15];

/// `speaker_info` for the synthesised HDMI sink.
///
/// How the byte encodes a speaker layout is unestablished, so the
/// value is CellGov's pick like the rest of the descriptor.
const HDMI_SPEAKER_INFO: u8 = 0x4F;

/// `speaker_info` for the AV-multi port: one bit, against the one
/// stereo audio block that port reports.
const AVMULTI_SPEAKER_INFO: u8 = 1;

/// `gamma` the HDMI descriptor reports.
///
/// The field's scale is unestablished, so the value is CellGov's pick
/// like the rest of the descriptor.
const HDMI_GAMMA: u32 = 100;

/// The fixed HDMI 0 monitor: a 27-inch 16:9 1080p panel.
///
/// CellGov synthesises these bytes; on a console they come from the
/// EDID of the attached display. The descriptor names an ordinary
/// HDTV: the CEA modes and nothing else, 1080p60 native, Rec.709
/// colour. An EDID pass-through behaviour mode reports only the
/// monitor type.
pub(super) fn hdmi_monitor_info(avport: u8, behavior: u8) -> [u8; av::PS3AV_MONITOR_INFO_LEN] {
    use av::monitor_info as mi;
    let mut m = [0u8; av::PS3AV_MONITOR_INFO_LEN];
    if behavior != av::PS3AV_HDMI_BEHAVIOR_NORMAL
        && behavior & av::PS3AV_HDMI_BEHAVIOR_EDID_PASS != 0
    {
        m[mi::MONITOR_TYPE_OFFSET] = if behavior & av::PS3AV_HDMI_BEHAVIOR_DVI != 0 {
            av::PS3AV_MONITOR_TYPE_DVI
        } else {
            av::PS3AV_MONITOR_TYPE_HDMI
        };
        return m;
    }
    m[mi::AVPORT_OFFSET] = avport;
    put_bytes(&mut m, mi::MONITOR_ID_OFFSET, &MONITOR_ID);
    m[mi::MONITOR_TYPE_OFFSET] = av::PS3AV_MONITOR_TYPE_HDMI;
    put_bytes(&mut m, mi::MONITOR_NAME_OFFSET, MONITOR_NAME);
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
        let at = mi::RES_TABLE_OFFSET + i * mi::RES_TABLE_SIZE;
        put_bytes(&mut m, at, &bits.to_be_bytes());
        put_bytes(&mut m, at + mi::RES_WORD_SIZE, &nat.to_be_bytes());
    }
    m[mi::CS_RGB_OFFSET] = av::PS3AV_CS_SUPPORTED
        | av::PS3AV_RGB_SELECTABLE_QUANTIZATION_RANGE
        | av::PS3AV_12BIT_COLOR;
    m[mi::CS_YUV444_OFFSET] = av::PS3AV_CS_SUPPORTED | av::PS3AV_12BIT_COLOR;
    m[mi::CS_YUV422_OFFSET] = av::PS3AV_CS_SUPPORTED;
    m[mi::COLORIMETRY_OFFSET] = av::PS3AV_COLORIMETRY_XVYCC_601
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
        put_bytes(
            &mut m,
            mi::COLOR_COORD_OFFSET + i * mi::COLOR_COORD_SIZE,
            &v.to_be_bytes(),
        );
    }
    put_bytes(&mut m, mi::GAMMA_OFFSET, &HDMI_GAMMA.to_be_bytes());
    m[mi::SUPPORTED_AI_OFFSET] = 1;
    m[mi::SPEAKER_INFO_OFFSET] = HDMI_SPEAKER_INFO;
    let audio: [(u8, u8, u8, u8); 7] = [
        (av::PS3AV_MON_INFO_AUDIO_TYPE_LPCM, 8, 0x7F, 0x07),
        (av::PS3AV_MON_INFO_AUDIO_TYPE_AC3, 8, 0x7F, 0xFF),
        (av::PS3AV_MON_INFO_AUDIO_TYPE_AAC, 8, 0x7F, 0xFF),
        (av::PS3AV_MON_INFO_AUDIO_TYPE_DTS, 8, 0x7F, 0xFF),
        (av::PS3AV_MON_INFO_AUDIO_TYPE_DDP, 8, 0x7F, 0xFF),
        (av::PS3AV_MON_INFO_AUDIO_TYPE_DTS_HD, 8, 0x7F, 0xFF),
        (av::PS3AV_MON_INFO_AUDIO_TYPE_DOLBY_THD, 8, 0x7F, 0xFF),
    ];
    put_bytes(
        &mut m,
        mi::NUM_AUDIO_BLOCK_OFFSET,
        &(audio.len() as u16).to_be_bytes(),
    );
    for (i, (ty, ch, fs, sbit)) in audio.into_iter().enumerate() {
        let at = mi::AUDIO_BLOCK_OFFSET + i * mi::AUDIO_BLOCK_SIZE;
        put_bytes(&mut m, at, &[ty, ch, fs, sbit]);
    }
    // 60 cm by 34 cm: a 27-inch panel at 16:9.
    put_bytes(&mut m, mi::HOR_SCREEN_SIZE_OFFSET, &60u16.to_be_bytes());
    put_bytes(&mut m, mi::VER_SCREEN_SIZE_OFFSET, &34u16.to_be_bytes());
    m[mi::CONTENT_TYPES_OFFSET] = 0b1111;

    // The 3D blocks at `RES_3D_OFFSET` stay zero: a plain HDTV carries
    // no 3D timing.
    m
}

/// The AV-multi port: every resolution, all three colour spaces, one
/// stereo LPCM block.
pub(super) fn avmulti_monitor_info() -> [u8; av::PS3AV_MONITOR_INFO_LEN] {
    use av::monitor_info as mi;
    let mut m = [0u8; av::PS3AV_MONITOR_INFO_LEN];
    m[mi::AVPORT_OFFSET] = av::PS3AV_AVPORT_AVMULTI_0 as u8;
    m[mi::MONITOR_TYPE_OFFSET] = av::PS3AV_MONITOR_TYPE_AVMULTI;
    // res_60, res_50 and res_vesa carry every bit; res_other stays
    // zero. The loop fills only the bit words, so no table names a
    // native timing.
    for table in [0, 1, 3] {
        let at = mi::RES_TABLE_OFFSET + table * mi::RES_TABLE_SIZE;
        put_bytes(&mut m, at, &u32::MAX.to_be_bytes());
    }
    m[mi::CS_RGB_OFFSET] = av::PS3AV_CS_SUPPORTED;
    m[mi::CS_YUV444_OFFSET] = av::PS3AV_CS_SUPPORTED;
    m[mi::CS_YUV422_OFFSET] = av::PS3AV_CS_SUPPORTED;
    m[mi::SPEAKER_INFO_OFFSET] = AVMULTI_SPEAKER_INFO;
    put_bytes(&mut m, mi::NUM_AUDIO_BLOCK_OFFSET, &1u16.to_be_bytes());
    put_bytes(
        &mut m,
        mi::AUDIO_BLOCK_OFFSET,
        &[av::PS3AV_MON_INFO_AUDIO_TYPE_LPCM, 2, 127, 7],
    );
    m
}

/// HDCP key list body: `(ksv_cnt u32, ksv[cnt][5])` padded to four
/// bytes, or a bare zero count when HDCP is off.
pub(super) fn ksv_list_body(behavior: u8) -> Vec<u8> {
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
