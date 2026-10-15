//! Holds `sys_spu_thread_write_ls` and `sys_spu_thread_read_ls` to the
//! `spu_thread_ls` capture from the reference console. The test makes
//! the calls of the probe, in its order, through the dispatch. The fake
//! runtime runs no SPU: it holds the thread's local store, which the
//! test fills as the thread does and into which it lands each write,
//! and the test reports the thread's exit as the runtime does.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{console_payload, payload_u32, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::{classify, Lv2Request};
use cellgov_ps3_abi::lv2::syscall::{SPU_THREAD_READ_LS, SPU_THREAD_WRITE_LS};

const PROBE: &str = "spu_thread_ls";
const CALLER: UnitId = UnitId::new(0);
const SPU: UnitId = UnitId::new(9);
/// Group C's two SPUs.
const SPU_C0: UnitId = UnitId::new(10);
const SPU_C1: UnitId = UnitId::new(11);
/// An out pointer the console leaves unmapped: the low page.
const UNMAPPED_PTR: u32 = 0x10;
const NO_SUCH_ID: u32 = 0xFFFF_FFFF;
const IMG_PTR: u32 = 0x300;
const VALUE_PTR: u32 = 0x800;
/// Where the probe's thread keeps the 32 bytes it fills with 0x00..0x1F.
const BUF: u64 = 0x3000;
const LS_SIZE: u64 = 0x4_0000;
/// The SELF control flags every probe EBOOT carries: root permission.
const PROBE_CONTROL_FLAGS1: u32 = 0x4000_0000;

struct Probe {
    host: Lv2Host,
    rt: FakeRuntime,
}

impl Probe {
    fn new() -> Self {
        let mut mem = GuestMemory::new(0x4000);
        // The kernel record image open writes: type KERNEL, image id 1.
        mem.apply_commit(
            ByteRange::new(GuestAddr::new(u64::from(IMG_PTR)), 8).unwrap(),
            &[0, 0, 0, 1, 0, 0, 0, 1],
        )
        .unwrap();
        let mut host = Lv2Host::new();
        host.set_control_flags1(PROBE_CONTROL_FLAGS1);
        host.content_store_mut().register(b"/spu.elf", vec![0xAA]);
        Self {
            host,
            rt: FakeRuntime::with_memory(mem).with_writable_at(u64::from(UNMAPPED_PTR), false),
        }
    }

    /// Dispatch `request`; land its local-store writes and return its
    /// answer and the 64-bit word it wrote, if any.
    fn call(&mut self, request: Lv2Request) -> (u32, Option<u64>) {
        let d = self.host.dispatch(request, CALLER, &self.rt);
        let (code, effects) = match d {
            Lv2Dispatch::Immediate { code, effects } => (code as u32, effects),
            Lv2Dispatch::RegisterSpu { code, .. } => (code as u32, vec![]),
            Lv2Dispatch::SpuGroupSuspension { .. } => (0, vec![]),
            other => panic!("unexpected dispatch {other:?}"),
        };
        let mut word = None;
        for effect in effects {
            match effect {
                Effect::SpuLocalStoreWrite {
                    target, lsa, bytes, ..
                } => {
                    let start = lsa as usize;
                    self.rt.local_store_mut(target)[start..start + bytes.len()]
                        .copy_from_slice(&bytes);
                }
                Effect::SharedWriteIntent { bytes, .. } => {
                    let raw = bytes.bytes();
                    word = Some(match raw.len() {
                        8 => u64::from_be_bytes(raw.try_into().unwrap()),
                        _ => u64::from(u32::from_be_bytes(raw[..4].try_into().unwrap())),
                    });
                }
                _ => {}
            }
        }
        (code, word)
    }

    /// The call the probe makes, classified from its argument words as
    /// the runtime does.
    fn syscall(&mut self, number: u64, args: [u64; 4]) -> (u32, Option<u64>) {
        let mut words = [0; 8];
        words[..4].copy_from_slice(&args);
        self.call(classify(number, &words))
    }

    fn write_ls(&mut self, thread_id: u32, lsa: u64, value: u64, size: u64) -> u32 {
        let thread = u64::from(thread_id);
        self.syscall(SPU_THREAD_WRITE_LS, [thread, lsa, value, size])
            .0
    }

    /// The answer, and the value word split hi / lo; an unwritten word
    /// reads as the probe's pre-fill.
    fn read_ls_to(&mut self, thread_id: u32, lsa: u64, ptr: u32, size: u64) -> [u32; 3] {
        let args = [u64::from(thread_id), lsa, u64::from(ptr), size];
        let (code, word) = self.syscall(SPU_THREAD_READ_LS, args);
        let word = word.unwrap_or(0xEEEE_EEEE_EEEE_EEEE);
        [code, (word >> 32) as u32, word as u32]
    }

    fn read_ls(&mut self, thread_id: u32, lsa: u64, size: u64) -> [u32; 3] {
        self.read_ls_to(thread_id, lsa, VALUE_PTR, size)
    }

    /// Initialize slot `thread_num` of `group_id`; returns the thread id.
    fn initialize(&mut self, group_id: u32, thread_num: u32) -> u32 {
        let (code, thread) = self.call(Lv2Request::SpuThreadInitialize {
            thread_ptr: 0x500,
            group_id,
            thread_num,
            img_ptr: IMG_PTR,
            attr_ptr: 0,
            arg_ptr: 0,
        });
        assert_eq!(code, 0, "initialize");
        thread.expect("the thread id") as u32
    }

    fn create(&mut self, num_threads: u32) -> u32 {
        let (code, gid) = self.call(Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x400,
            num_threads,
            priority: 100,
            attr_ptr: 0,
        });
        assert_eq!(code, 0, "create");
        gid.expect("the group id") as u32
    }

    /// The runtime registers the SPU of slot `slot` that a start created.
    fn register(&mut self, unit: UnitId, group_id: u32, slot: u32) {
        self.host
            .thread_groups_mut()
            .record_spu(unit, group_id, slot)
            .expect("the runtime registers the SPU");
    }
}

#[test]
fn write_ls_and_read_ls_answer_as_the_console_does() {
    let payload = console_payload(PROBE);
    let word = |w: usize| payload_u32(&payload, 4 * w);
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");

    let mut p = Probe::new();
    let gid = p.create(1);
    let t = p.initialize(gid, 0);

    let mut got = vec![0u32; 2];
    got.push(p.write_ls(t, BUF, 0, 4));
    got.push(p.read_ls(t, BUF, 4)[0]);
    got.push(p.write_ls(NO_SUCH_ID, BUF, 0, 4));
    got.push(p.read_ls(NO_SUCH_ID, BUF, 4)[0]);
    got.push(p.write_ls(NO_SUCH_ID, LS_SIZE, 0, 4));
    got.push(p.write_ls(NO_SUCH_ID, BUF, 0, 3));

    assert_eq!(
        p.call(Lv2Request::SpuThreadGroupStart { group_id: gid }).0,
        0
    );
    p.register(SPU, gid, 0);
    // The thread fills its buffer with 0x00..0x1F before it reports.
    for (i, byte) in p.rt.local_store_mut(SPU)[BUF as usize..][..32]
        .iter_mut()
        .enumerate()
    {
        *byte = i as u8;
    }

    got.extend([
        p.write_ls(t, LS_SIZE, 0, 4),
        p.write_ls(t, BUF + 1, 0, 2),
        p.write_ls(t, BUF + 2, 0, 4),
        p.write_ls(t, BUF + 4, 0, 8),
        p.write_ls(t, BUF, 0, 3),
        p.write_ls(t, BUF, 0, 0),
        p.write_ls(t, BUF, 0, 16),
        p.write_ls(t, BUF, 0xA1, 1),
        p.write_ls(t, BUF + 2, 0xB2B3, 2),
        p.write_ls(t, BUF + 4, 0xC4C5_C6C7, 4),
        p.write_ls(t, BUF + 8, 0xD8D9_DADB_DCDD_DEDF, 8),
        p.write_ls(t, BUF + 16, 0xFFFF_FFFF_E0E1_E2E3, 4),
        p.write_ls(t, BUF + 20, 0x1FF, 1),
    ]);
    got.extend(p.read_ls(t, BUF, 1));
    got.extend(p.read_ls(t, BUF + 2, 2));
    got.extend(p.read_ls(t, BUF + 4, 4));
    got.extend(p.read_ls(t, BUF + 8, 8));
    got.extend(p.read_ls(t, BUF + 24, 8));
    got.push(p.read_ls_to(t, BUF, 0, 4)[0]);
    got.push(p.read_ls(t, BUF + 1, 2)[0]);
    got.push(p.read_ls(t, LS_SIZE, 1)[0]);
    got.push(p.read_ls(t, BUF, 3)[0]);
    got.push(p.read_ls(t, LS_SIZE - 1, 1)[0]);

    got.push(
        p.call(Lv2Request::SpuThreadGroupSuspend { group_id: gid })
            .0,
    );
    got.push(p.write_ls(t, BUF + 12, 0x77, 1));
    let [code, _, lo] = p.read_ls(t, BUF + 12, 1);
    got.extend([code, lo]);
    got.push(p.call(Lv2Request::SpuThreadGroupResume { group_id: gid }).0);

    got.push(
        p.call(Lv2Request::SpuThreadWriteMb {
            thread_id: t,
            value: 1,
        })
        .0,
    );
    p.host
        .spu_thread_exit(SPU, 0)
        .expect("the thread exits after it puts its buffer back");
    let (code, _) = p.call(Lv2Request::SpuThreadGroupJoin {
        group_id: gid,
        cause_ptr: 0x600,
        status_ptr: 0x604,
    });
    assert_eq!(code, 0, "join");
    // The fake runtime commits no write, so the cause and status come
    // from the record the join read.
    let (cause, status) = p.host.spu_group_join_result(gid).expect("finished");
    got.extend([cause, status]);
    let ls = &p.rt.local_store_mut(SPU)[BUF as usize..][..32];
    got.extend(
        ls.chunks(4)
            .map(|c| u32::from_be_bytes(c.try_into().unwrap())),
    );

    got.push(p.write_ls(t, BUF, 0, 4));
    got.push(p.read_ls(t, BUF, 4)[0]);
    assert_eq!(p.call(Lv2Request::SpuThreadGroupDestroy { id: gid }).0, 0);
    got.push(p.write_ls(t, BUF, 0, 4));
    got.push(p.read_ls(t, BUF, 4)[0]);
    got.extend([0, 0, 0]);
    assert_eq!(got.len(), 64);

    // Group C: t0 runs and waits; t1 exits at once with status 7.
    let gid = p.create(2);
    let t0 = p.initialize(gid, 0);
    let t1 = p.initialize(gid, 1);
    got.push(p.read_ls_to(t0, BUF, 0, 4)[0]);
    assert_eq!(
        p.call(Lv2Request::SpuThreadGroupStart { group_id: gid }).0,
        0
    );
    p.register(SPU_C0, gid, 0);
    p.register(SPU_C1, gid, 1);
    p.host
        .spu_thread_exit(SPU_C1, 7)
        .expect("t1 exits while t0 runs");
    got.push(
        p.call(Lv2Request::SpuThreadGetExitStatus {
            thread_id: t1,
            status_ptr: 0x608,
        })
        .0,
    );
    got.push(p.write_ls(t1, BUF, 0x55, 1));
    let [code, _, lo] = p.read_ls(t1, BUF, 1);
    got.extend([code, lo]);
    got.push(p.write_ls(t0, 1 << 32 | BUF, 0x66, 1));
    got.push(p.write_ls(t0, BUF, 0x66, 1 << 32 | 1));
    got.push(p.read_ls(t0, 1 << 32 | BUF, 1)[0]);
    got.push(p.read_ls_to(t0, BUF, UNMAPPED_PTR, 1)[0]);
    got.push(p.read_ls_to(t0, BUF, VALUE_PTR + 1, 1)[0]);
    got.push(p.read_ls_to(NO_SUCH_ID, BUF, 0, 4)[0]);
    got.push(p.read_ls_to(NO_SUCH_ID, LS_SIZE, 0, 4)[0]);
    got.push(
        p.call(Lv2Request::SpuThreadWriteMb {
            thread_id: t0,
            value: 1,
        })
        .0,
    );
    p.host.spu_thread_exit(SPU_C0, 0).expect("t0 exits");
    assert_eq!(
        p.call(Lv2Request::SpuThreadGroupJoin {
            group_id: gid,
            cause_ptr: 0x600,
            status_ptr: 0x604,
        })
        .0,
        0
    );
    let (cause, status) = p.host.spu_group_join_result(gid).expect("finished");
    got.extend([cause, status]);
    got.push(p.call(Lv2Request::SpuThreadGroupDestroy { id: gid }).0);
    assert_eq!(got.len(), 80);

    for (w, &value) in got.iter().enumerate().skip(2) {
        assert_eq!(
            value,
            word(w),
            "w{w}: 0x{value:08x} vs console 0x{:08x}",
            word(w)
        );
    }
}
