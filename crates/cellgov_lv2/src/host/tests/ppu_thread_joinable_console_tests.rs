//! The JOINABLE flag of `_sys_ppu_thread_create` held to the
//! `lv2_ppu_thread_joinable` capture from the reference console. The
//! classifier reads each call from the probe's literal syscall number and
//! arguments, and the test compares its code with the captured word.

use cellgov_event::UnitId;

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{
    console_payload, opd_runtime, payload_u32, primary_attrs, seed_primary_ppu, FakeRuntime,
};
use crate::host::Lv2Host;
use crate::ppu_thread::PpuThreadId;
use crate::request::classify;

const CALLER: UnitId = UnitId::new(0);
/// The SELF control flags every probe EBOOT carries: root permission.
const PROBE_CONTROL_FLAGS1: u32 = 0x4000_0000;
const PARAM: u64 = 0x200;
const ID_PTR: u64 = 0x1000;
const PRIO_PTR: u64 = 0x2000;
const VALUE_PTR: u64 = 0x2010;

struct Probe {
    host: Lv2Host,
    rt: FakeRuntime,
    next_unit: u64,
}

impl Probe {
    fn new() -> Self {
        let mut host = Lv2Host::new();
        host.set_control_flags1(PROBE_CONTROL_FLAGS1);
        seed_primary_ppu(&mut host, CALLER);
        Self {
            host,
            rt: opd_runtime(PARAM as u32, 0x10_0000, 0x10_0100),
            next_unit: 1,
        }
    }

    fn syscall(&mut self, number: u64, args: &[u64]) -> Lv2Dispatch {
        let mut all = [0u64; 8];
        all[..args.len()].copy_from_slice(args);
        self.host.dispatch(classify(number, &all), CALLER, &self.rt)
    }

    fn code(&mut self, number: u64, args: &[u64]) -> u32 {
        match self.syscall(number, args) {
            Lv2Dispatch::Immediate { code, .. } => code as u32,
            Lv2Dispatch::PpuThreadStart { .. } => 0,
            other => panic!("syscall {number}: unexpected dispatch {other:?}"),
        }
    }

    /// A 52 with the probe's arguments and `flags`, entered in the table
    /// as the runtime enters it. Returns the thread id.
    fn create(&mut self, flags: u64) -> u64 {
        let joinable = match self.syscall(52, &[ID_PTR, PARAM, 0, 0, 1000, 0x4000, flags, 0]) {
            Lv2Dispatch::PpuThreadCreate { joinable, .. } => joinable,
            other => panic!("52: unexpected dispatch {other:?}"),
        };
        let unit = UnitId::new(self.next_unit);
        self.next_unit += 1;
        self.host
            .ppu_threads_mut()
            .create_unstarted(unit, primary_attrs(), joinable)
            .expect("thread create")
            .raw()
    }

    fn exit(&mut self, thread: u64, value: u64) {
        self.host
            .ppu_threads_mut()
            .mark_finished(PpuThreadId::new(thread), value);
    }
}

#[test]
fn the_joinable_flag_answers_as_on_the_console() {
    let payload = console_payload("lv2_ppu_thread_joinable");
    let word = |w: usize| payload_u32(&payload, 4 * w);
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");
    assert_eq!((word(4), word(7), word(8)), (1, 0, 1), "E ran, then exited");
    let mut p = Probe::new();

    // E: created with flags 0, joined while it runs.
    let e = p.create(0);
    assert_eq!(p.code(53, &[e, 0]), word(3), "w3: 53 E");
    let join = p.syscall(44, &[e, VALUE_PTR]);
    assert_eq!(
        join,
        Lv2Dispatch::immediate(u64::from(word(5)) | 0xFFFF_FFFF_0000_0000),
        "w5: the join answers at once and writes nothing (w6 0x{:08x})",
        word(6)
    );
    p.exit(e, 0x0E);
    assert_eq!(
        p.code(44, &[e, VALUE_PTR]),
        word(9),
        "w9: 44 after E exited"
    );
    assert_eq!(
        p.code(48, &[e, PRIO_PTR]),
        word(10),
        "w10: 48 after E exited"
    );
    assert_eq!(p.code(53, &[e, 0]), word(11), "w11: 53 after E exited");

    // G: created joinable, found after it exits, then joined.
    let g = p.create(1);
    assert_eq!(p.code(53, &[g, 0]), 0, "53 G");
    p.exit(g, 0x6);
    assert_eq!(
        p.code(48, &[g, PRIO_PTR]),
        word(13),
        "w13: 48 after G exited"
    );
    match p.syscall(44, &[g, VALUE_PTR]) {
        Lv2Dispatch::Immediate { code: 0, effects } => {
            let cellgov_effects::Effect::SharedWriteIntent { bytes, .. } = &effects[0] else {
                panic!(
                    "44 G: expected the exit value's write, got {:?}",
                    effects[0]
                );
            };
            let v = bytes.bytes();
            assert_eq!(v.len(), 8, "the join writes a u64");
            let low = u32::from_be_bytes([v[4], v[5], v[6], v[7]]);
            assert_eq!(low << 16, word(14), "w14: the joined value");
        }
        other => panic!("44 G: unexpected dispatch {other:?}"),
    }

    // H: created with flags 0, joined before it starts.
    let h = p.create(0);
    assert_eq!(
        p.code(44, &[h, VALUE_PTR]),
        word(15),
        "w15: 44 before H starts"
    );
}
