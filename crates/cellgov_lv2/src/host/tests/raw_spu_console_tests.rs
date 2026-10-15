//! Holds the Raw SPU calls to the `spu_raw_lifecycle` capture from the
//! reference console. The test makes the probe's calls, in its order,
//! through the dispatch, and registers each created Raw SPU's unit as
//! the runtime does.
//!
//! Words 38-47 come from a run of the Raw SPU through its problem-state
//! window, which CellGov does not map yet, and so do the words that
//! words 48, 50, 53 and 59 read; the test compares the answers of those
//! calls and not the words. Word 52 reads the window too.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::syscall::{
    RAW_SPU_CREATE, RAW_SPU_CREATE_INTERRUPT_TAG, RAW_SPU_DESTROY, RAW_SPU_GET_INT_MASK,
    RAW_SPU_GET_INT_STAT, RAW_SPU_GET_SPU_CFG, RAW_SPU_READ_PUINT_MB, RAW_SPU_RECOVER_PAGE_FAULT,
    RAW_SPU_SET_INT_MASK, RAW_SPU_SET_INT_STAT, RAW_SPU_SET_SPU_CFG, SPU_INITIALIZE,
};

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{console_payload, payload_u32, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::classify;

const PROBE: &str = "spu_raw_lifecycle";
const CALLER: UnitId = UnitId::new(0);
/// The id the probe names when it means no Raw SPU.
const NO_SUCH_RAW: u64 = 63;
/// Where the probe keeps the words the calls write.
const OUT_PTR: u64 = 0x800;
/// An out pointer the console leaves unmapped: the low page.
const UNMAPPED_PTR: u64 = 0x10;
/// The SELF control flags every probe EBOOT carries: root permission.
const PROBE_CONTROL_FLAGS1: u32 = 0x4000_0000;
/// What the probe's out words hold before a call writes them.
const UNTOUCHED: u64 = 0xEEEE_EEEE_EEEE_EEEE;

struct Probe {
    host: Lv2Host,
    rt: FakeRuntime,
}

impl Probe {
    fn new() -> Self {
        let mut host = Lv2Host::new();
        host.set_control_flags1(PROBE_CONTROL_FLAGS1);
        Self {
            host,
            rt: FakeRuntime::new(0x1000).with_writable_at(UNMAPPED_PTR, false),
        }
    }

    /// Make the call `number` with `args`, classified as the runtime
    /// classifies it. Returns the answer, the word written or
    /// [`UNTOUCHED`], and the word's width in bytes (0 for none). A
    /// create registers the Raw SPU's unit as the runtime does.
    fn syscall(&mut self, number: u64, args: [u64; 4]) -> (u32, u64, usize) {
        let mut words = [0; 8];
        words[..4].copy_from_slice(&args);
        let d = self
            .host
            .dispatch(classify(number, &words), CALLER, &self.rt);
        let (code, effects) = match d {
            Lv2Dispatch::Immediate { code, effects } => (code as u32, effects),
            Lv2Dispatch::RawSpuCreate { id, effects } => {
                self.host
                    .record_raw_spu(id, UnitId::new(100 + u64::from(id)));
                (0, effects)
            }
            Lv2Dispatch::RawSpuDestroy { .. } => (0, vec![]),
            other => panic!("unexpected dispatch {other:?}"),
        };
        let (word, width) = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::SharedWriteIntent { bytes, .. } => {
                    let raw = bytes.bytes();
                    Some(match raw.len() {
                        8 => (u64::from_be_bytes(raw.try_into().unwrap()), 8),
                        _ => (u64::from(u32::from_be_bytes(raw.try_into().unwrap())), 4),
                    })
                }
                _ => None,
            })
            .unwrap_or((UNTOUCHED, 0));
        (code, word, width)
    }

    /// The answer, and the low word of what the call wrote. A call that
    /// answers `CELL_OK` writes a word `width` bytes wide: the probe
    /// takes a 64-bit word's value from its low half and a 32-bit word's
    /// from the word itself, so the capture fixes each width.
    fn out_call(&mut self, number: u64, args: [u64; 4], width: usize) -> [u32; 2] {
        let (code, word, written) = self.syscall(number, args);
        if code == 0 {
            assert_eq!(written, width, "call {number}: out word width");
        }
        [code, word as u32]
    }

    fn create(&mut self) -> [u32; 2] {
        self.out_call(RAW_SPU_CREATE, [OUT_PTR, 0, 0, 0], 4)
    }

    fn code(&mut self, number: u64, args: [u64; 4]) -> u32 {
        self.syscall(number, args).0
    }
}

#[test]
fn raw_spu_calls_answer_as_the_console_does() {
    let payload = console_payload(PROBE);
    let word = |w: usize| payload_u32(&payload, 4 * w);
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");

    let mut p = Probe::new();
    let mut got = vec![None; 80];
    let mut put = |w: usize, value: u32| got[w] = Some(value);

    let [code, id] = p.create();
    put(2, code);
    put(3, id);
    put(4, p.code(SPU_INITIALIZE, [6, 2, 0, 0]));
    let [code, a] = p.create();
    put(5, code);
    put(6, a);
    let [code, b] = p.create();
    put(7, code);
    put(8, b);
    put(9, p.create()[0]);
    put(61, p.code(RAW_SPU_CREATE, [0, 0, 0, 0]));
    put(10, p.code(SPU_INITIALIZE, [6, 1, 0, 0]));
    put(11, p.code(RAW_SPU_DESTROY, [u64::from(b), 0, 0, 0]));
    put(12, p.code(RAW_SPU_DESTROY, [u64::from(b), 0, 0, 0]));
    put(13, p.code(RAW_SPU_DESTROY, [NO_SUCH_RAW, 0, 0, 0]));
    put(14, p.code(RAW_SPU_CREATE, [0, 0, 0, 0]));
    let [code, id] = p.create();
    put(62, code);
    put(63, id);
    assert_eq!(p.code(RAW_SPU_DESTROY, [u64::from(id), 0, 0, 0]), 0);

    let a = u64::from(a);
    let [code, tag0] = p.out_call(RAW_SPU_CREATE_INTERRUPT_TAG, [a, 0, 0, OUT_PTR], 4);
    put(15, code);
    put(17, p.code(RAW_SPU_CREATE_INTERRUPT_TAG, [a, 0, 0, OUT_PTR]));
    put(18, p.code(RAW_SPU_CREATE_INTERRUPT_TAG, [a, 1, 0, OUT_PTR]));
    let [code, tag2] = p.out_call(RAW_SPU_CREATE_INTERRUPT_TAG, [a, 2, 0, OUT_PTR], 4);
    put(19, code);
    put(
        21,
        p.code(RAW_SPU_CREATE_INTERRUPT_TAG, [NO_SUCH_RAW, 0, 0, OUT_PTR]),
    );

    put(22, p.code(RAW_SPU_SET_INT_MASK, [a, 0, 0x7, 0]));
    let [code, mask] = p.out_call(RAW_SPU_GET_INT_MASK, [a, 0, OUT_PTR, 0], 8);
    put(23, code);
    put(24, mask);
    put(25, p.code(RAW_SPU_SET_INT_MASK, [a, 2, 0xF, 0]));
    put(
        26,
        p.out_call(RAW_SPU_GET_INT_MASK, [a, 2, OUT_PTR, 0], 8)[1],
    );
    put(27, p.code(RAW_SPU_SET_INT_MASK, [a, 1, 1, 0]));
    put(28, p.code(RAW_SPU_SET_INT_MASK, [NO_SUCH_RAW, 0, 0, 0]));
    let [code, stat] = p.out_call(RAW_SPU_GET_INT_STAT, [a, 0, OUT_PTR, 0], 8);
    put(29, code);
    put(30, stat);

    put(31, p.code(RAW_SPU_SET_SPU_CFG, [a, 3, 0, 0]));
    let [code, cfg] = p.out_call(RAW_SPU_GET_SPU_CFG, [a, OUT_PTR, 0, 0], 4);
    put(32, code);
    put(33, cfg);
    put(34, p.code(RAW_SPU_SET_SPU_CFG, [a, 4, 0, 0]));
    put(
        35,
        p.out_call(RAW_SPU_GET_SPU_CFG, [a, OUT_PTR, 0, 0], 4)[1],
    );
    put(36, p.code(RAW_SPU_RECOVER_PAGE_FAULT, [a, 0, 0, 0]));
    put(
        37,
        p.code(RAW_SPU_RECOVER_PAGE_FAULT, [NO_SUCH_RAW, 0, 0, 0]),
    );

    let [code, mb] = p.out_call(RAW_SPU_READ_PUINT_MB, [a, OUT_PTR, 0, 0], 4);
    put(57, code);
    // The console reads 0x21000000 here: whatever the physical SPU's
    // register last held. CellGov reads 0 from a Raw SPU whose mailbox
    // never held a message.
    assert_eq!(mb, 0, "an untouched PU interrupt mailbox reads 0");

    let no_such = NO_SUCH_RAW;
    put(64, p.code(RAW_SPU_SET_INT_MASK, [no_such, 1, 0, 0]));
    put(65, p.code(RAW_SPU_GET_INT_MASK, [no_such, 1, OUT_PTR, 0]));
    put(
        66,
        p.code(RAW_SPU_CREATE_INTERRUPT_TAG, [no_such, 1, 0, OUT_PTR]),
    );
    put(67, p.code(RAW_SPU_GET_INT_MASK, [a, 0, 0, 0]));
    put(68, p.code(RAW_SPU_CREATE, [UNMAPPED_PTR, 0, 0, 0]));
    let [code, id] = p.create();
    put(69, code);
    put(70, id);
    assert_eq!(p.code(RAW_SPU_DESTROY, [u64::from(id), 0, 0, 0]), 0);
    put(71, p.code(RAW_SPU_READ_PUINT_MB, [a, UNMAPPED_PTR, 0, 0]));
    put(72, p.code(RAW_SPU_GET_SPU_CFG, [a, 0, 0, 0]));
    put(73, p.code(RAW_SPU_SET_INT_MASK, [a, 3, 0, 0]));
    let [code, c] = p.create();
    assert_eq!(code, 0, "Raw SPU C");
    let c = u64::from(c);
    put(74, p.code(RAW_SPU_CREATE_INTERRUPT_TAG, [c, 0, 0, 0]));
    put(75, p.code(RAW_SPU_CREATE_INTERRUPT_TAG, [c, 0, 0, OUT_PTR]));
    let [code, stat] = p.out_call(RAW_SPU_GET_INT_STAT, [c, 2, OUT_PTR, 0], 8);
    put(76, code);
    put(77, stat);
    assert_eq!(p.code(RAW_SPU_DESTROY, [c, 0, 0, 0]), 0);
    // The probe makes these calls after the run; their answers do not
    // depend on it, and the words they read do, except word 55: a clear
    // of every bit leaves the mailbox threshold bit, whatever ran.
    put(
        48,
        p.out_call(RAW_SPU_GET_INT_STAT, [a, 2, OUT_PTR, 0], 8)[0],
    );
    put(
        50,
        p.out_call(RAW_SPU_READ_PUINT_MB, [a, OUT_PTR, 0, 0], 4)[0],
    );
    put(54, p.code(RAW_SPU_SET_INT_STAT, [a, 2, u64::MAX, 0]));
    put(
        55,
        p.out_call(RAW_SPU_GET_INT_STAT, [a, 2, OUT_PTR, 0], 8)[1],
    );
    put(
        59,
        p.out_call(RAW_SPU_READ_PUINT_MB, [a, OUT_PTR, 0, 0], 4)[0],
    );
    put(56, p.code(RAW_SPU_DESTROY, [a, 0, 0, 0]));

    // The console's tags count with a kernel object counter the rest of
    // the system shares; the kind byte and the spacing are the tag's
    // own.
    for (w, tag) in [(16, tag0), (20, tag2)] {
        assert_eq!(tag >> 24, word(w) >> 24, "w{w}: tag kind");
    }
    assert_eq!(tag2 - tag0, word(20) - word(16), "tags created in turn");

    let compared = got.iter().filter(|value| value.is_some()).count();
    assert_eq!(compared, 58, "every call the test makes is compared");
    for (w, value) in got.iter().enumerate() {
        if let Some(value) = *value {
            assert_eq!(
                value,
                word(w),
                "w{w}: 0x{value:08x} vs console 0x{:08x}",
                word(w)
            );
        }
    }
}
