use super::*;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::WRITE_VIRTUAL_UART;
use cellgov_time::GuestTicks;

/// The dispatcher manager's port [OFW-4.93 lv1 a:0x5B7E7C-0x5B7E80].
const PORT: u64 = 10;

/// The `sc` of the 4.93 kernel's write
/// [OFW-4.93 lv2_kernel a:0x8000000000016fd8].
const KERNEL_SC: u64 = 0x8000_0000_0001_6fd8;

/// The header of the 4.93 kernel's second request in CellGov's boot: tag
/// 1, service 0x5000, and 0x40 request and reply bytes
/// [OFW-4.93 lv2_kernel a:0x80000000002121f4-0x80000000002121fc]
/// [OFW-4.93 lv2_kernel a:0x8000000000228178].
const RANDOM_HEADER: [u8; 16] = [0, 0, 0, 1, 0, 0, 0x50, 0, 0, 0, 0, 0x40, 0, 0, 0, 0x40];

/// The 0x40 request bytes of that request: the request id 0x5003 and the
/// service id 0x5000 at +0 and +8
/// [OFW-4.93 lv2_kernel a:0x8000000000227fb0-0x8000000000227fc0], and the
/// doubleword that the caller gives at +32
/// [OFW-4.93 lv2_kernel a:0x800000000022839c-0x80000000002283b4].
fn random_body() -> Vec<u8> {
    let mut body = vec![0u8; 0x40];
    body[..8].copy_from_slice(&0x5003u64.to_be_bytes());
    body[8..16].copy_from_slice(&0x5000u64.to_be_bytes());
    body[32..40].copy_from_slice(&0x1070_0000_3100_0001u64.to_be_bytes());
    body
}

fn write(pc: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(0),
        number: WRITE_VIRTUAL_UART,
        inputs: [PORT, 0, 0x40, 0, 0, 0, 0, 0],
        pc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(31_050),
    }
}

#[test]
fn request_0x5003_at_a_firmware_with_no_image_read_gets_the_refusal_for_its_service() {
    assert_eq!(
        dispatcher_reply(
            "4.92",
            &PendingRequest::empty(PORT).with(&RANDOM_HEADER),
            &random_body()
        ),
        Err(NoReply::Unsourced(Unsourced {
            service: 0x5000,
            request: Some(0x5003),
            part: REQUEST_PART,
        }))
    );
}

#[test]
fn the_refusal_names_hypercall_163_the_service_id_the_request_id_and_the_part() {
    let unsourced = NoReply::Unsourced(Unsourced {
        service: 0x5000,
        request: Some(0x5002),
        part: REQUEST_PART,
    });
    let refusal = HypercallRefusal::DispatcherRequest {
        unit: UnitId::new(0),
        pc: KERNEL_SC,
        service: 0x5000,
        request: Some(0x5002),
        part: REQUEST_PART,
    };
    assert_eq!(
        unsourced.refusal(&write(KERNEL_SC)),
        HypercallDisposition::Refused(refusal)
    );
    assert_eq!(refusal.number(), Some(163));
    assert_eq!(
        refusal.to_string(),
        "hypercall 163 from UnitId(0) at pc 0x8000000000016fd8: CellGov does not model the reply \
         to request 0x5002 of service 0x5000 on the dispatcher manager's port: no source gives a \
         reply of this service, which the dispatcher manager relays from the service or gives from \
         its own state"
    );
}

#[test]
fn the_refusal_of_a_request_with_no_request_id_says_so() {
    let refusal = HypercallRefusal::DispatcherRequest {
        unit: UnitId::new(0),
        pc: KERNEL_SC,
        service: 0x19000,
        request: None,
        part: SERVICE_REQUEST_PART,
    };
    assert_eq!(
        refusal.to_string(),
        "hypercall 163 from UnitId(0) at pc 0x8000000000016fd8: CellGov does not model the reply \
         to a request with no request id of service 0x19000 on the dispatcher manager's port: no \
         source gives a reply of this request, which a service of lv1 gives"
    );
}

/// `RANDOM_HEADER` with `request` request bytes and `reply` reply bytes,
/// then `request` request bytes that start with the request id `id`.
fn random_request_with(id: u64, request: u32, reply: u32) -> (PendingRequest, Vec<u8>) {
    let mut header = RANDOM_HEADER;
    header[8..12].copy_from_slice(&request.to_be_bytes());
    header[12..].copy_from_slice(&reply.to_be_bytes());
    let mut body = vec![0u8; request as usize];
    if let Some(first) = body.get_mut(..8) {
        first.copy_from_slice(&id.to_be_bytes());
    }
    (PendingRequest::empty(PORT).with(&header), body)
}

#[test]
fn request_0x5003_with_counts_other_than_0x40_gets_the_refusal_of_its_sizes() {
    for (request, reply) in [(0x38, 0x38), (0x40, 0x38), (0x38, 0x40), (8, 0x40)] {
        let (pending, body) = random_request_with(0x5003, request, reply);
        assert_eq!(
            dispatcher_reply("4.93", &pending, &body),
            Err(NoReply::Unsourced(Unsourced {
                service: 0x5000,
                request: Some(0x5003),
                part: SIZE_PART,
            })),
            "{request:#x} {reply:#x}"
        );
    }
}

#[test]
fn another_request_of_service_0x5000_gets_the_refusal_for_its_service() {
    let (pending, body) = random_request_with(0x5002, 0x40, 0x40);
    assert_eq!(
        dispatcher_reply("4.93", &pending, &body),
        Err(NoReply::Unsourced(Unsourced {
            service: 0x5000,
            request: Some(0x5002),
            part: REQUEST_PART,
        }))
    );
}

#[test]
fn a_request_of_service_0x5000_with_fewer_than_8_request_bytes_has_no_request_id() {
    for request in [0, 7] {
        let (pending, body) = random_request_with(0x5003, request, 0x40);
        assert_eq!(
            dispatcher_reply("4.93", &pending, &body),
            Err(NoReply::Unsourced(Unsourced {
                service: 0x5000,
                request: None,
                part: REQUEST_PART,
            })),
            "{request:#x}"
        );
    }
}
