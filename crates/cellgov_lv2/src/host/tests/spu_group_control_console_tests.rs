//! Holds group terminate, the exit status of a thread and the argument
//! replacement to the `spu_group_control` capture from the reference
//! console. The test makes the calls of the probe, in its order,
//! through the dispatch.

use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{console_payload, payload_u32, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::Lv2Request;

const PROBE: &str = "spu_group_control";
const NO_SUCH_ID: u32 = 0xFFFF_FFFF;
const IMG_PTR: u32 = 0x300;
const ARG_PTR: u32 = 0x200;
const NEW_ARG_PTR: u32 = 0x240;
const STATUS_PTR: u32 = 0x600;
const CALLER: UnitId = UnitId::new(0);

/// The value `#define <name>` sets in one of the probe's sources.
fn define(source: &str, name: &str) -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(PROBE)
        .join(source);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let value = text
        .lines()
        .find_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some("#define") && words.next() == Some(name))
                .then(|| words.next())
                .flatten()
        })
        .unwrap_or_else(|| panic!("{source}: no #define {name}"));
    let hex = value.trim_end_matches("ull").trim_end_matches('u');
    u64::from_str_radix(hex.trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("{name} = {value}: {e}"))
}

fn put(mem: &mut GuestMemory, addr: u32, bytes: &[u8]) {
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(u64::from(addr)), bytes.len() as u64).unwrap(),
        bytes,
    )
    .unwrap();
}

/// The answer the guest reads in r3.
fn code(d: &Lv2Dispatch) -> u32 {
    match d {
        Lv2Dispatch::Immediate { code, .. } => *code as u32,
        Lv2Dispatch::SpuGroupTerminate { .. } => 0,
        other => panic!("unexpected dispatch {other:?}"),
    }
}

struct Probe {
    host: Lv2Host,
    rt: FakeRuntime,
}

impl Probe {
    fn call(&mut self, request: Lv2Request) -> Lv2Dispatch {
        self.host.dispatch(request, CALLER, &self.rt)
    }

    fn terminate(&mut self, group_id: u32, value: u32) -> u32 {
        code(&self.call(Lv2Request::SpuThreadGroupTerminate {
            group_id,
            value: value as i32,
        }))
    }

    fn exit_status(&mut self, thread_id: u32, status_ptr: u32) -> Lv2Dispatch {
        self.call(Lv2Request::SpuThreadGetExitStatus {
            thread_id,
            status_ptr,
        })
    }

    fn set_argument(&mut self, thread_id: u32, arg_ptr: u32) -> u32 {
        code(&self.call(Lv2Request::SpuThreadSetArgument { thread_id, arg_ptr }))
    }

    /// Create a one-thread group and initialize its slot 0; returns the
    /// group and thread ids.
    fn create_group(&mut self) -> (u32, u32) {
        let created = self.call(Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x400,
            num_threads: 1,
            priority: 100,
            attr_ptr: 0,
        });
        let Lv2Dispatch::Immediate { code: 0, effects } = &created else {
            panic!("expected a created group, got {created:?}");
        };
        let [cellgov_effects::Effect::SharedWriteIntent { bytes, .. }] = effects.as_slice() else {
            panic!("expected one id write, got {effects:?}");
        };
        let group = u32::from_be_bytes(bytes.bytes().try_into().expect("4 bytes"));
        let d = self.call(Lv2Request::SpuThreadInitialize {
            thread_ptr: 0x500,
            group_id: group,
            thread_num: 0,
            img_ptr: IMG_PTR,
            attr_ptr: 0,
            arg_ptr: ARG_PTR,
        });
        assert_eq!(code(&d), 0);
        (group, group * crate::thread_group::MAX_SLOTS_PER_GROUP)
    }
}

#[test]
fn terminate_exit_status_and_set_argument_answer_as_the_console_does() {
    let payload = console_payload(PROBE);
    let word = |w: usize| payload_u32(&payload, 4 * w);
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");

    let mut host = Lv2Host::new();
    host.content_store_mut()
        .register(b"/spu.elf", vec![0xAA, 0xBB]);
    let mut mem = GuestMemory::new(0x4000);
    put(&mut mem, 0x100, b"/spu.elf\0");
    put(&mut mem, IMG_PTR, &[0, 0, 0, 1, 0, 0, 0, 1]);
    let old_args = [
        2,
        0x1000,
        define("ppu/main.c", "OLD_ARG2"),
        define("ppu/main.c", "OLD_ARG3"),
    ];
    let new_args = [
        2,
        0x1000,
        define("ppu/main.c", "NEW_ARG2"),
        define("ppu/main.c", "NEW_ARG3"),
    ];
    put(
        &mut mem,
        ARG_PTR,
        &old_args
            .iter()
            .flat_map(|a| a.to_be_bytes())
            .collect::<Vec<_>>(),
    );
    put(
        &mut mem,
        NEW_ARG_PTR,
        &new_args
            .iter()
            .flat_map(|a| a.to_be_bytes())
            .collect::<Vec<_>>(),
    );
    let rt = FakeRuntime::with_memory(mem);
    let mut p = Probe { host, rt };
    let opened = p.call(Lv2Request::SpuImageOpen {
        img_ptr: IMG_PTR,
        path_ptr: 0x100,
    });
    assert_eq!(code(&opened), 0);

    // Group A, initialized and not started.
    let (group, thread) = p.create_group();
    let mut answers = vec![
        p.terminate(NO_SUCH_ID, 0),
        p.terminate(group, 0),
        code(&p.exit_status(NO_SUCH_ID, STATUS_PTR)),
        code(&p.exit_status(thread, STATUS_PTR)),
        code(&p.exit_status(thread, 0)),
        p.set_argument(NO_SUCH_ID, NEW_ARG_PTR),
        p.set_argument(NO_SUCH_ID, 0),
        p.set_argument(thread, 0),
    ];

    // Group A running, then terminated.
    let started = p.call(Lv2Request::SpuThreadGroupStart { group_id: group });
    assert!(matches!(started, Lv2Dispatch::RegisterSpu { code: 0, .. }));
    p.host.record_spu(UnitId::new(10), group, 0).unwrap();
    answers.push(p.set_argument(thread, NEW_ARG_PTR));
    answers.push(code(&p.exit_status(thread, STATUS_PTR)));
    let value = define("ppu/main.c", "TERMINATE_STATUS") as u32;
    answers.push(p.terminate(group, value));
    let (cause, status) = p.host.spu_group_join_result(group).expect("joinable");
    answers.extend([cause, status]);
    answers.push(code(&p.exit_status(thread, STATUS_PTR)));
    answers.push(p.terminate(group, 0));
    answers.push(p.set_argument(thread, NEW_ARG_PTR));
    let destroyed = p.call(Lv2Request::SpuThreadGroupDestroy { id: group });
    assert_eq!(code(&destroyed), 0);

    // Group B: new arguments before the start, then an exit status.
    let (group, thread) = p.create_group();
    answers.push(p.set_argument(thread, NEW_ARG_PTR));
    let started = p.call(Lv2Request::SpuThreadGroupStart { group_id: group });
    let Lv2Dispatch::RegisterSpu { inits, code: 0, .. } = started else {
        panic!("expected RegisterSpu, got {started:?}");
    };
    let unit = UnitId::new(11);
    p.host.record_spu(unit, group, 0).unwrap();
    let exit = define("spu/main.c", "EXIT_STATUS") as u32;
    p.host.spu_thread_exit(unit, exit).unwrap();
    let (cause, status) = p.host.spu_group_join_result(group).expect("joinable");
    answers.extend([cause, status]);
    let d = p.exit_status(thread, STATUS_PTR);
    answers.push(code(&d));
    let Lv2Dispatch::Immediate { effects, .. } = &d else {
        panic!("expected Immediate, got {d:?}");
    };
    let [cellgov_effects::Effect::SharedWriteIntent { bytes, .. }] = effects.as_slice() else {
        panic!("expected one status write, got {effects:?}");
    };
    answers.push(u32::from_be_bytes(
        bytes.bytes().try_into().expect("4 bytes"),
    ));
    answers.push(code(&p.exit_status(thread, 0)));

    let console: Vec<u32> = (2..24).map(word).collect();
    assert_eq!(answers, console, "words 2 to 23");

    // The C runtime of the SPU passes `main` only its first three
    // argument registers, so the toolchain sets the fourth argument
    // word in the capture. `arg2` is the third u64 of the block.
    let arg2 = u64::from(word(24)) << 32 | u64::from(word(25));
    assert_eq!(inits[&0].args[2], arg2, "the block group B started with");
}
