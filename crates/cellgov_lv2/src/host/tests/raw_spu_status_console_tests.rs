//! The interrupt-status reads of the `spu_raw_status` capture from the
//! reference console: after a stop, the class 0 and 2 reads answer
//! CELL_OK and the console refuses the process its class 1 read. The status
//! words the reads write follow the run; the `console_raw_status` test
//! in `cellgov_spu` compares those.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::syscall::{RAW_SPU_CREATE, RAW_SPU_GET_INT_STAT, SPU_INITIALIZE};

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{console_payload, payload_u32, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::classify;

const CALLER: UnitId = UnitId::new(0);
const OUT_PTR: u64 = 0x800;

/// The answer to call `number` with `args`, classified as the runtime
/// classifies it; a create registers the Raw SPU's unit and answers its
/// id.
fn call(host: &mut Lv2Host, rt: &FakeRuntime, number: u64, args: [u64; 4]) -> u64 {
    let mut words = [0; 8];
    words[..4].copy_from_slice(&args);
    match host.dispatch(classify(number, &words), CALLER, rt) {
        Lv2Dispatch::Immediate { code, .. } => code,
        Lv2Dispatch::RawSpuCreate { id, .. } => {
            host.record_raw_spu(id, UnitId::new(100 + u64::from(id)));
            u64::from(id)
        }
        other => panic!("call {number}: unexpected dispatch {other:?}"),
    }
}

#[test]
fn the_interrupt_status_reads_answer_as_the_console_does() {
    let payload = console_payload("spu_raw_status");
    let word = |w: usize| payload_u32(&payload, 4 * w);
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1000);
    call(&mut host, &rt, SPU_INITIALIZE, [6, 1, 0, 0]);
    let id = call(&mut host, &rt, RAW_SPU_CREATE, [OUT_PTR, 0, 0, 0]);
    for class in 0..3u64 {
        let code = call(
            &mut host,
            &rt,
            RAW_SPU_GET_INT_STAT,
            [id, class, OUT_PTR, 0],
        );
        assert_eq!(code as u32, word(6 + class as usize), "class {class}");
    }
}
