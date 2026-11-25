use super::*;

/// The dispatcher manager's port [OFW-4.93 lv1 a:0x5B7E7C-0x5B7E80].
const PORT: u64 = 10;

/// The counts of the kernel's class for request 0x19004 at 4.93
/// [OFW-4.93 lv2_kernel a:0x8000000000229f34].
const IDENTITY_BYTES: u32 = 0x30;

/// A header for service 0x19000 with `request` request bytes and `reply`
/// reply bytes [OFW-4.93 lv2_kernel a:0x80000000002121f4-0x80000000002121fc],
/// then `request` request bytes that start with the request id `id` and
/// the service id 0x19000
/// [OFW-4.93 lv2_kernel a:0x800000000022989c-0x80000000002298ac].
fn identity_request_with(id: u64, request: u32, reply: u32) -> (PendingRequest, Vec<u8>) {
    let mut header = [0u8; 16];
    header[..4].copy_from_slice(&3u32.to_be_bytes());
    header[4..8].copy_from_slice(&0x19000u32.to_be_bytes());
    header[8..12].copy_from_slice(&request.to_be_bytes());
    header[12..].copy_from_slice(&reply.to_be_bytes());
    let mut body = vec![0u8; request as usize];
    if let Some(first) = body.get_mut(..8) {
        first.copy_from_slice(&id.to_be_bytes());
    }
    if let Some(service) = body.get_mut(8..16) {
        service.copy_from_slice(&0x19000u64.to_be_bytes());
    }
    (PendingRequest::empty(PORT).with(&header), body)
}

fn identity_refusal(part: &'static str) -> Result<Vec<u8>, NoReply> {
    Err(NoReply::Unsourced(Unsourced {
        service: 0x19000,
        request: Some(0x19004),
        part,
    }))
}

#[test]
fn request_0x19004_with_counts_other_than_0x30_gets_the_refusal_of_its_sizes() {
    for (request, reply) in [(0x38, 0x38), (0x30, 0x38), (0x38, 0x30), (8, 0x30)] {
        let (pending, body) = identity_request_with(0x19004, request, reply);
        assert_eq!(
            dispatcher_reply("4.93", &pending, &body),
            identity_refusal(SIZE_PART),
            "{request:#x} {reply:#x}"
        );
    }
}

#[test]
fn request_0x19004_at_a_firmware_with_no_image_read_gets_the_refusal_for_its_service() {
    let (pending, body) = identity_request_with(0x19004, IDENTITY_BYTES, IDENTITY_BYTES);
    assert_eq!(
        dispatcher_reply("4.92", &pending, &body),
        identity_refusal(SERVICE_REQUEST_PART)
    );
}
