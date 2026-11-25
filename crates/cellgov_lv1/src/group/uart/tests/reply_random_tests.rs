use super::*;

/// The dispatcher manager's port [OFW-4.93 lv1 a:0x5B7E7C-0x5B7E80].
const PORT: u64 = 10;

/// The header of the 4.93 kernel's second request in CellGov's boot: tag
/// 1, service 0x5000, and 0x40 request and reply bytes
/// [OFW-4.93 lv2_kernel a:0x80000000002121f4-0x80000000002121fc]
/// [OFW-4.93 lv2_kernel a:0x8000000000228178].
const RANDOM_HEADER: [u8; 16] = [0, 0, 0, 1, 0, 0, 0x50, 0, 0, 0, 0, 0x40, 0, 0, 0, 0x40];

/// The 0x40 request bytes of that request: the request id 0x5003 and the
/// service id 0x5000 at +0 and +8
/// [OFW-4.93 lv2_kernel a:0x8000000000227fb0-0x8000000000227fc0], the
/// doubleword that the caller gives at +32
/// [OFW-4.93 lv2_kernel a:0x800000000022839c-0x80000000002283b4], and the
/// byte value 0xA5 in each other byte.
fn random_body() -> Vec<u8> {
    let mut body = vec![0xA5u8; 0x40];
    body[..8].copy_from_slice(&0x5003u64.to_be_bytes());
    body[8..16].copy_from_slice(&0x5000u64.to_be_bytes());
    body[32..40].copy_from_slice(&0x1070_0000_3100_0001u64.to_be_bytes());
    body
}

/// The status word is 0 because lv1 writes the random data bytes only on
/// the path where its worker returns 0, and the kernel keeps its seed
/// only for that status [OFW-4.93 lv1 a:0x50B534-0x50B558]
/// [OFW-4.93 lv2_kernel a:0x8000000000228470-0x800000000022847c]. The data
/// bytes are the ASCII label that the doc of [`SYNTHETIC_RANDOM_DATA`]
/// gives.
#[test]
fn the_kernel_s_request_0x5003_of_service_0x5000_gets_status_0_and_the_synthetic_random_data() {
    let request = random_body();
    let reply = dispatcher_reply(
        "4.93",
        &PendingRequest::empty(PORT).with(&RANDOM_HEADER),
        &request,
    )
    .expect("a reply");
    let mut expected = vec![0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x40];
    let mut echoed = request;
    echoed[16..20].copy_from_slice(&[0, 0, 0, 0]);
    echoed[40..64].copy_from_slice(b"CellGov synthetic random");
    expected.extend_from_slice(&echoed);
    assert_eq!(reply, expected);
}

#[test]
fn the_synthetic_seed_bytes_hold_more_than_one_byte_value() {
    let seed = &SYNTHETIC_RANDOM_DATA[..20];
    assert!(
        seed.iter().any(|&byte| byte != seed[0]),
        "the 20 seed bytes are each {:#04x}",
        seed[0]
    );
}
