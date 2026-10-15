//! The SPU thread signal, configuration and inbound mailbox calls held
//! to the `spu_signal_mbox_edges` capture from the reference console.
//! The test makes the probe's calls, in its order, through the dispatch.
//! The fake runtime runs no SPU: the test reports the thread's exit
//! with the message the mailbox write delivered, as the probe's SPU
//! exits with the value it read.

use cellgov_effects::Effect;
use cellgov_event::UnitId;

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{console_payload, payload_u32, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::Lv2Request;

const PROBE: &str = "spu_signal_mbox_edges";
const CALLER: UnitId = UnitId::new(0);
/// The unit the runtime registers for the group's SPU.
const SPU: UnitId = UnitId::new(7);
const CFG_PTR: u32 = 0x100;
const STATUS_PTR: u32 = 0x108;

/// The value `#define <name>` sets in the probe's PPU source.
fn define(name: &str) -> u32 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(PROBE)
        .join("ppu/main.c");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let value = text
        .lines()
        .find_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some("#define") && words.next() == Some(name))
                .then(|| words.next())
                .flatten()
        })
        .unwrap_or_else(|| panic!("no #define {name}"));
    u32::from_str_radix(value.trim_end_matches('u').trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("{name} = {value}: {e}"))
}

struct Probe {
    host: Lv2Host,
    rt: FakeRuntime,
}

impl Probe {
    fn call(&mut self, request: Lv2Request) -> Lv2Dispatch {
        self.host.dispatch(request, CALLER, &self.rt)
    }

    /// The code of an immediate answer, and the bytes of each write.
    fn answer(&mut self, request: Lv2Request) -> (u32, Vec<Vec<u8>>) {
        match self.call(request) {
            Lv2Dispatch::Immediate { code, effects } => (
                code as u32,
                effects
                    .iter()
                    .filter_map(|e| match e {
                        Effect::SharedWriteIntent { bytes, .. } => Some(bytes.bytes().to_vec()),
                        _ => None,
                    })
                    .collect(),
            ),
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    fn set_cfg(&mut self, thread_id: u32, value: u64) -> u32 {
        self.answer(Lv2Request::SpuThreadSetSpuCfg { thread_id, value })
            .0
    }

    /// The code, then the configuration's high and low words when the
    /// call wrote it.
    fn get_cfg(&mut self, thread_id: u32, value_ptr: u32) -> (u32, Option<[u32; 2]>) {
        let (code, writes) = self.answer(Lv2Request::SpuThreadGetSpuCfg {
            thread_id,
            value_ptr,
        });
        let words = writes.first().map(|b| {
            [
                u32::from_be_bytes(b[0..4].try_into().expect("4 bytes")),
                u32::from_be_bytes(b[4..8].try_into().expect("4 bytes")),
            ]
        });
        (code, words)
    }

    fn write_snr(&mut self, thread_id: u32, number: u32, value: u32) -> u32 {
        self.answer(Lv2Request::SpuThreadWriteSnr {
            thread_id,
            number,
            value,
        })
        .0
    }
}

#[test]
fn signal_configuration_and_mailbox_calls_answer_as_on_the_console() {
    let payload = console_payload(PROBE);
    let word = |w: usize| payload_u32(&payload, 4 * w);
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");
    let no_such_id = define("NO_SUCH_ID");
    let snr = define("SNR_VALUE");
    let mbox = define("MBOX_VALUE");

    let mut host = Lv2Host::new();
    let handle = host
        .content_store_mut()
        .register(b"/spu.elf", vec![0x7F, b'E', b'L', b'F']);
    let group = host.thread_groups_mut().create(1).expect("a group id");
    host.thread_groups_mut()
        .initialize_thread(group, 0, handle, [0; 4])
        .expect("slot 0");
    let thread = group * crate::thread_group::MAX_SLOTS_PER_GROUP;
    let mut p = Probe {
        host,
        rt: FakeRuntime::new(0x4000),
    };

    // Thread initialized, group not started.
    let mut got = vec![0, 0];
    got.push(p.set_cfg(no_such_id, 4));
    got.push(p.set_cfg(no_such_id, 0));
    got.push(p.set_cfg(thread, 4));
    got.push(p.set_cfg(thread, 3));
    let (code, cfg) = p.get_cfg(thread, CFG_PTR);
    let [high, low] = cfg.expect("the configuration");
    got.extend([code, high, low]);
    got.push(p.get_cfg(no_such_id, 0).0);
    got.push(p.get_cfg(thread, 0).0);
    got.push(p.write_snr(no_such_id, 2, snr));
    got.push(p.write_snr(thread, 0, snr));
    got.push(p.write_snr(thread, 2, snr));
    got.push(
        p.answer(Lv2Request::SpuThreadWriteMb {
            thread_id: no_such_id,
            value: mbox,
        })
        .0,
    );

    // Group running.
    let started = p.call(Lv2Request::SpuThreadGroupStart { group_id: group });
    assert!(matches!(started, Lv2Dispatch::RegisterSpu { code: 0, .. }));
    p.host
        .record_spu(SPU, group, 0)
        .expect("the runtime registers the SPU");
    let (code, cfg) = p.get_cfg(thread, CFG_PTR);
    got.extend([code, cfg.expect("the configuration")[1]]);
    got.push(p.set_cfg(thread, 1));
    let (code, cfg) = p.get_cfg(thread, CFG_PTR);
    got.extend([code, cfg.expect("the configuration")[1]]);
    let Lv2Dispatch::Immediate { code, effects } = p.call(Lv2Request::SpuThreadWriteMb {
        thread_id: thread,
        value: mbox,
    }) else {
        panic!("expected Immediate");
    };
    got.push(code as u32);
    let [Effect::MailboxSend {
        mailbox, message, ..
    }] = effects.as_slice()
    else {
        panic!("expected one mailbox send, got {effects:?}");
    };
    assert_eq!(
        mailbox.raw(),
        SPU.raw(),
        "the thread's SPU takes the message"
    );
    p.host
        .spu_thread_exit(SPU, message.raw())
        .expect("the thread exits with the message it read");
    let (cause, status) = p.host.spu_group_join_result(group).expect("joinable");
    got.extend([cause, status]);
    let (code, writes) = p.answer(Lv2Request::SpuThreadGetExitStatus {
        thread_id: thread,
        status_ptr: STATUS_PTR,
    });
    got.push(code);
    got.push(u32::from_be_bytes(
        writes.first().expect("the status")[0..4]
            .try_into()
            .expect("4 bytes"),
    ));

    // Group ended.
    got.push(p.write_snr(thread, 0, snr));
    let (code, cfg) = p.get_cfg(thread, CFG_PTR);
    got.extend([code, cfg.expect("the configuration")[1]]);
    got.push(p.answer(Lv2Request::SpuThreadGroupDestroy { id: group }).0);
    assert_eq!(got.len(), 29);

    for (w, &value) in got.iter().enumerate() {
        assert_eq!(
            value,
            word(w),
            "w{w}: 0x{value:08x} vs console 0x{:08x}",
            word(w)
        );
    }
}
