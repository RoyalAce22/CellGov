//! Holds the priority group create accepts, and the answers of
//! `sys_spu_thread_group_set_priority` and
//! `sys_spu_thread_group_get_priority`, to the `spu_group_priority`
//! capture from the reference console. The test makes the calls of the
//! probe, in its order, through the dispatch.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::spu::{group_attr, group_type};

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{console_payload, payload_u32, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::Lv2Request;

const PROBE: &str = "spu_group_priority";
const USER_PROBE: &str = "spu_group_priority_user";
const CALLER: UnitId = UnitId::new(0);
const NO_SUCH_ID: u32 = 0xFFFF_FFFF;
/// The SELF control flags every probe EBOOT carries: the probe's
/// packaging signs it with root permission, so the console answers
/// as it does for a root process.
const PROBE_CONTROL_FLAGS1: u32 = 0x4000_0000;

/// One group attribute per type the probe creates.
const CONTEXT: u32 = 0x800;
const EXCLUSIVE: u32 = 0x810;
const COOP: u32 = 0x820;

struct Probe {
    host: Lv2Host,
    rt: FakeRuntime,
}

/// What a call answered: its return code, and for a get the priority it
/// wrote.
fn code(d: &Lv2Dispatch) -> (u32, Option<u32>) {
    match d {
        Lv2Dispatch::Immediate { code, effects } => {
            let written = match effects.as_slice() {
                [Effect::SharedWriteIntent { bytes, .. }] => Some(u32::from_be_bytes(
                    bytes.bytes().try_into().expect("4 bytes"),
                )),
                _ => None,
            };
            (*code as u32, written)
        }
        other => panic!("unexpected dispatch {other:?}"),
    }
}

impl Probe {
    fn new(control_flags1: u32) -> Self {
        let mut mem = GuestMemory::new(0x4000);
        for (attr, ty) in [
            (CONTEXT, group_type::NORMAL),
            (EXCLUSIVE, group_type::EXCLUSIVE_NON_CONTEXT),
            (COOP, group_type::COOPERATE_WITH_SYSTEM),
        ] {
            let at = u64::from(attr + group_attr::TYPE_OFFSET);
            mem.apply_commit(
                ByteRange::new(GuestAddr::new(at), 4).unwrap(),
                &ty.to_be_bytes(),
            )
            .unwrap();
        }
        let mut host = Lv2Host::new();
        host.set_control_flags1(control_flags1);
        Self {
            host,
            rt: FakeRuntime::with_memory(mem),
        }
    }

    fn call(&mut self, request: Lv2Request) -> (u32, Option<u32>) {
        code(&self.host.dispatch(request, CALLER, &self.rt))
    }

    /// Create a group; returns the code and the id it wrote.
    fn create(&mut self, attr_ptr: u32, num_threads: u32, priority: i32) -> (u32, Option<u32>) {
        self.call(Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x400,
            num_threads,
            priority,
            attr_ptr,
        })
    }

    fn set(&mut self, group_id: u32, priority: i32) -> u32 {
        self.call(Lv2Request::SpuThreadGroupSetPriority { group_id, priority })
            .0
    }

    /// get_priority; the read-back word is 0xEEEEEEEE when the call wrote
    /// nothing, as the probe's buffer reads.
    fn get(&mut self, group_id: u32, priority_ptr: u32) -> (u32, u32) {
        let (code, written) = self.call(Lv2Request::SpuThreadGroupGetPriority {
            group_id,
            priority_ptr,
        });
        (code, written.unwrap_or(0xEEEE_EEEE))
    }
}

#[test]
fn group_priority_answers_as_the_console_does() {
    answers_as_the_console_does(PROBE, PROBE_CONTROL_FLAGS1);
}

/// The same calls from a process without root permission, as the
/// `spu_group_priority_user` probe makes them: its EBOOT is signed with
/// control_flags1 0, as a retail title is, so priority 15 is refused.
#[test]
fn group_priority_answers_a_process_without_root_as_the_console_does() {
    answers_as_the_console_does(USER_PROBE, 0);
}

/// Make the probe's calls, in its order, from a process whose SELF
/// carries `control_flags1`, and hold each answer to `probe`'s capture.
fn answers_as_the_console_does(probe: &str, control_flags1: u32) {
    let payload = console_payload(probe);
    let word = |w: usize| payload_u32(&payload, 4 * w);
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");

    let mut p = Probe::new(control_flags1);
    let mut got = vec![0u32; 2];
    got.push(p.create(CONTEXT, 1, 15).0);
    let (rc, c) = p.create(CONTEXT, 1, 16);
    got.push(rc);
    let (rc, d) = p.create(CONTEXT, 1, 255);
    got.push(rc);
    got.push(p.create(CONTEXT, 1, 256).0);
    got.push(p.create(CONTEXT, 1, -1).0);
    let (rc, x) = p.create(EXCLUSIVE, 1, 0);
    got.push(rc);
    let (rc, coop) = p.create(COOP, 2, 0);
    got.push(rc);
    let [c, d, x, coop] = [c, d, x, coop].map(|id| id.expect("a created group"));
    const OUT: u32 = 0x600;

    let (rc, v) = p.get(c, OUT);
    got.extend([rc, v]);
    got.push(p.set(c, 200));
    let (rc, v) = p.get(c, OUT);
    got.extend([rc, v]);
    got.push(p.set(c, 15));
    got.push(p.set(c, 256));
    got.push(p.set(c, -1));

    got.push(p.set(NO_SUCH_ID, 100));
    got.push(p.set(NO_SUCH_ID, 15));
    got.push(p.get(NO_SUCH_ID, OUT).0);
    got.push(p.get(c, 0).0);

    got.push(p.set(x, 100));
    let (rc, v) = p.get(x, OUT);
    got.extend([rc, v]);
    got.push(p.set(coop, 100));
    let (rc, v) = p.get(coop, OUT);
    got.extend([rc, v]);
    let (rc, v) = p.get(d, OUT);
    got.extend([rc, v]);
    assert_eq!(got.len(), 29);

    for (w, &value) in got.iter().enumerate().skip(2) {
        assert_eq!(
            value,
            word(w),
            "w{w}: 0x{value:08x} vs console 0x{:08x}",
            word(w)
        );
    }
}
