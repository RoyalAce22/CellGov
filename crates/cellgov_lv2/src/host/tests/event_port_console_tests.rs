//! The event port calls and the tryreceive batch answer the
//! `lv2_event_port_answers` probe as the reference console does, word
//! for word.
//!
//! Each call goes through `classify` from the raw syscall number and
//! argument registers the probe passes, and the write effects land in a
//! byte map that stands for guest memory.

use std::collections::BTreeMap;

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};

use super::*;
use crate::host::test_support::{console_payload, payload_u32, seed_primary_ppu, FakeRuntime};
use crate::request::classify;
use cellgov_ps3_abi::lv2::syscall;

const PROBE: &str = "lv2_event_port_answers";

const IPC_KEY: u64 = 0x0000_C0DE_1476_0001;

const ID_PTR: u64 = 0x100;
const COUNT_PTR: u64 = 0x200;
const EVENTS_PTR: u64 = 0x400;
const QUEUE_ATTR_PTR: u64 = 0x800;

struct Probe {
    host: Lv2Host,
    rt: FakeRuntime,
    src: UnitId,
    mem: BTreeMap<u64, u8>,
}

impl Probe {
    fn new() -> Self {
        let mut guest = GuestMemory::new(0x10000);
        // sys_event_queue_attribute_t: protocol FIFO, type PPU queue.
        let attr = [0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0];
        guest
            .apply_commit(
                ByteRange::new(GuestAddr::new(QUEUE_ATTR_PTR), 16).unwrap(),
                &attr,
            )
            .unwrap();
        let mut host = Lv2Host::new();
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        Self {
            host,
            rt: FakeRuntime::with_memory(guest),
            src,
            mem: BTreeMap::new(),
        }
    }

    /// One raw syscall: its return code, with its writes applied.
    fn sc(&mut self, num: u64, args: &[u64]) -> u32 {
        let mut regs = [0u64; 8];
        regs[..args.len()].copy_from_slice(args);
        let d = self.host.dispatch(classify(num, &regs), self.src, &self.rt);
        let (code, effects) = match d {
            Lv2Dispatch::Immediate { code, effects } => (code, effects),
            Lv2Dispatch::WakeAndReturn { code, effects, .. } => (code, effects),
            other => panic!("syscall {num}: expected an answer, got {other:?}"),
        };
        for effect in effects {
            if let cellgov_effects::Effect::SharedWriteIntent { range, bytes, .. } = effect {
                for (i, b) in bytes.bytes().iter().enumerate() {
                    self.mem.insert(range.start().raw() + i as u64, *b);
                }
            }
        }
        code as u32
    }

    /// The big-endian u32 at `addr`, reading an unwritten byte as 0xFF
    /// (the probe's fill).
    fn u32_at(&self, addr: u64) -> u32 {
        let b = |i: u64| *self.mem.get(&(addr + i)).unwrap_or(&0xFF);
        u32::from_be_bytes([b(0), b(1), b(2), b(3)])
    }

    fn port_create(&mut self, port_type: u64) -> (u32, u32) {
        let rc = self.sc(syscall::EVENT_PORT_CREATE, &[ID_PTR, port_type, 0]);
        (rc, self.u32_at(ID_PTR))
    }

    fn queue_create(&mut self, key: u64, size: u64) -> (u32, u32) {
        let rc = self.sc(
            syscall::EVENT_QUEUE_CREATE,
            &[ID_PTR, QUEUE_ATTR_PTR, key, size],
        );
        (rc, self.u32_at(ID_PTR))
    }
}

/// CellGov's 40 payload words.
fn cellgov_words() -> Vec<u32> {
    let mut p = Probe::new();
    let mut w = Vec::new();
    for t in 0..4 {
        let (rc, port) = p.port_create(t);
        w.push(rc);
        if rc == 0 {
            p.sc(syscall::EVENT_PORT_DESTROY, &[u64::from(port)]);
        }
    }
    let (rc, queue) = p.queue_create(IPC_KEY, 4);
    w.push(rc);
    let (rc, ipc) = p.port_create(3);
    w.push(rc);
    let ipc = u64::from(ipc);
    w.push(p.sc(syscall::EVENT_PORT_CONNECT_IPC, &[ipc, IPC_KEY]));
    w.push(p.sc(syscall::EVENT_PORT_CONNECT_IPC, &[ipc, IPC_KEY]));
    w.push(p.sc(syscall::EVENT_PORT_DISCONNECT, &[ipc]));
    w.push(p.sc(syscall::EVENT_PORT_CONNECT_IPC, &[ipc, 0]));
    w.push(p.sc(syscall::EVENT_PORT_CONNECT_IPC, &[ipc, IPC_KEY + 1]));
    let (rc, local) = p.port_create(1);
    w.push(rc);
    w.push(p.sc(
        syscall::EVENT_PORT_CONNECT_IPC,
        &[u64::from(local), IPC_KEY],
    ));
    w.push(p.sc(syscall::EVENT_PORT_DESTROY, &[u64::from(local)]));
    w.push(p.sc(syscall::EVENT_PORT_CONNECT_IPC, &[ipc, IPC_KEY]));
    w.push(p.sc(syscall::EVENT_PORT_DESTROY, &[ipc]));
    w.push(p.sc(syscall::EVENT_PORT_DISCONNECT, &[ipc]));
    w.push(p.sc(syscall::EVENT_PORT_DESTROY, &[ipc]));
    w.push(p.sc(syscall::EVENT_QUEUE_DESTROY, &[u64::from(queue), 0]));

    let (rc, cport) = p.port_create(1);
    w.push(rc);
    let cport = u64::from(cport);
    w.push(p.sc(syscall::EVENT_PORT_SEND, &[cport, 0x10, 0, 0]));
    let (rc, cqueue) = p.queue_create(0, 1);
    w.push(rc);
    w.push(p.sc(
        syscall::EVENT_PORT_CONNECT_LOCAL,
        &[cport, u64::from(cqueue)],
    ));
    w.push(p.sc(syscall::EVENT_PORT_SEND, &[cport, 0x11, 0, 0]));
    w.push(p.sc(syscall::EVENT_PORT_SEND, &[cport, 0x12, 0, 0]));

    let (rc, dqueue) = p.queue_create(0, 4);
    w.push(rc);
    let (rc, dport) = p.port_create(1);
    w.push(rc);
    let (dport, dqueue) = (u64::from(dport), u64::from(dqueue));
    w.push(p.sc(syscall::EVENT_PORT_CONNECT_LOCAL, &[dport, dqueue]));
    w.push(p.sc(syscall::EVENT_PORT_SEND, &[dport, 0x21, 0, 0]));
    w.push(p.sc(syscall::EVENT_PORT_SEND, &[dport, 0x22, 0, 0]));
    w.push(p.sc(
        syscall::EVENT_QUEUE_TRY_RECEIVE,
        &[dqueue, EVENTS_PTR, 3, COUNT_PTR],
    ));
    w.push(p.u32_at(COUNT_PTR));
    // sys_event_t is four u64s: data1 is at +8, its low word at +12.
    for i in 0..3 {
        w.push(p.u32_at(EVENTS_PTR + 32 * i + 12));
    }

    p.sc(syscall::EVENT_PORT_DISCONNECT, &[cport]);
    w.push(p.sc(syscall::EVENT_PORT_DESTROY, &[cport]));
    w.push(p.sc(syscall::EVENT_QUEUE_DESTROY, &[u64::from(cqueue), 0]));
    p.sc(syscall::EVENT_PORT_DISCONNECT, &[dport]);
    w.push(p.sc(syscall::EVENT_PORT_DESTROY, &[dport]));
    w.push(p.sc(syscall::EVENT_QUEUE_DESTROY, &[dqueue, 0]));
    w.push(0);
    w
}

#[test]
fn the_event_port_calls_answer_as_the_console_does() {
    let payload = console_payload(PROBE);
    let console: Vec<u32> = (0..40).map(|i| payload_u32(&payload, 4 * i)).collect();
    let cellgov = cellgov_words();
    for (i, (got, want)) in cellgov.iter().zip(&console).enumerate() {
        assert_eq!(got, want, "payload word {i}");
    }
    assert_eq!(cellgov.len(), console.len());
}
