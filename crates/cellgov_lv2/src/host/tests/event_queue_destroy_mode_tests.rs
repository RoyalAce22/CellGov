//! `sys_event_queue_destroy` with mode words other than 0 and FORCE
//! answers the `lv2_event_queue_destroy_mode` probe as the reference
//! console does: EINVAL, checked before the id, and the queue left in
//! place.

use super::*;
use crate::host::test_support::{
    console_payload, extract_write_u32, payload_u32, seed_primary_ppu, FakeRuntime,
};
use crate::request::Lv2Request;

const PROBE: &str = "lv2_event_queue_destroy_mode";

#[test]
fn unlisted_destroy_modes_answer_as_the_console_does() {
    let payload = console_payload(PROBE);
    assert_eq!(
        (payload_u32(&payload, 0), payload_u32(&payload, 4)),
        (0, 0),
        "the probe created its queue"
    );
    let console: Vec<u32> = (2..6).map(|i| payload_u32(&payload, 4 * i)).collect();

    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let id = match host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size: 16,
        },
        src,
        &rt,
    ) {
        Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let cellgov: Vec<u32> = [(id, 2), (id, 0x8000_0000), (0xDEAD_BEEF, 2), (id, 0)]
        .into_iter()
        .map(|(queue_id, mode)| {
            match host.dispatch(Lv2Request::EventQueueDestroy { queue_id, mode }, src, &rt) {
                Lv2Dispatch::Immediate { code, .. } => code as u32,
                other => panic!("expected Immediate, got {other:?}"),
            }
        })
        .collect();
    assert_eq!(cellgov, console, "steps 2..=5");
}
