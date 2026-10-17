//! The PRX start, stop and unload arms held to the reference console's
//! answers per module state: the `lv2_prx_states`,
//! `lv2_prx_states_more` and `lv2_prx_states_last` captures.
//!
//! Each probe loads a fresh copy of a firmware module for every
//! question. The replay registers a fresh module in its place and sends
//! the probe's calls in the probe's order, so a module's state follows
//! the same path on both sides.

use super::*;
use crate::host::test_support::console_payload;
use cellgov_ps3_abi::lv2::prx::start_stop_option::NO_ENTRY;
use cellgov_ps3_abi::lv2::syscall;

/// Where each raw call's option struct sits.
const OPT: u32 = 0x4000;
/// The probes seed `entry` and `entry2` so a write to either shows.
const ENTRY_SEED: u64 = 0x1111_1111_1111_1111;
const ENTRY2_SEED: u64 = 0x2222_2222_2222_2222;
const START: u64 = syscall::SYS_PRX_START_MODULE;
const STOP: u64 = syscall::SYS_PRX_STOP_MODULE;
const UNLOAD: u64 = syscall::SYS_PRX_UNLOAD_MODULE;

/// One capture's payload and the CellGov host replaying its calls.
struct Replay {
    payload: Vec<u8>,
    host: Lv2Host,
    loads: u32,
}

impl Replay {
    fn new(probe: &str, slots: usize) -> Self {
        let payload = console_payload(probe);
        assert_eq!(payload.len(), 8 * slots, "{probe}");
        Self {
            payload,
            host: Lv2Host::new(),
            loads: 0,
        }
    }

    /// The console's u64 at `slot`.
    fn word(&self, slot: usize) -> u64 {
        u64::from_be_bytes(
            self.payload[8 * slot..8 * slot + 8]
                .try_into()
                .expect("8 bytes"),
        )
    }

    /// A fresh module, as each `sysPrxLoadModule` in the probe loads one.
    fn load(&mut self) -> u32 {
        self.loads += 1;
        let stem = format!("libprobe{}", self.loads);
        let base = 0x0100_0000 + 0x10_0000 * self.loads;
        self.host.prx_registry_mut().register(
            stem.clone(),
            stem,
            base,
            base + 0x8_0000,
            base + 0x1000,
            None,
            None,
        )
    }

    /// One raw start or stop call with the probes' 0x28-byte option:
    /// its answer and the two entry words after it.
    fn raw(&mut self, number: u64, id: u32, cmd: u64, res: u64) -> (u64, [u64; 2]) {
        let image: Vec<u8> = [0x28, cmd, ENTRY_SEED, res, ENTRY2_SEED]
            .iter()
            .flat_map(|w: &u64| w.to_be_bytes())
            .collect();
        let mut mem = cellgov_mem::GuestMemory::new(0x10000);
        mem.apply_commit(
            ByteRange::new(cellgov_mem::GuestAddr::new(u64::from(OPT)), 0x28).unwrap(),
            &image,
        )
        .unwrap();
        let rt = FakeRuntime::with_memory(mem);
        let request = Lv2Request::Unsupported {
            number,
            args: [u64::from(id), 0, u64::from(OPT), 0, 0, 0, 0, 0],
        };
        let (code, effects) = match self.host.dispatch(request, UnitId::new(0), &rt) {
            Lv2Dispatch::Immediate { code, effects } => (code, effects),
            other => panic!("sc {number}: expected an immediate answer, got {other:?}"),
        };
        let mut entries = [ENTRY_SEED, ENTRY2_SEED];
        for effect in &effects {
            let Effect::SharedWriteIntent { range, bytes, .. } = effect else {
                panic!("sc {number}: unexpected effect {effect:?}");
            };
            let word = u64::from_be_bytes(bytes.bytes().try_into().expect("8 bytes"));
            match range.start().raw() - u64::from(OPT) {
                0x10 => entries[0] = word,
                0x20 => entries[1] = word,
                other => panic!("sc {number}: write at option offset {other:#x}"),
            }
        }
        (code, entries)
    }

    fn rc(&mut self, number: u64, id: u32, cmd: u64, res: u64) -> u64 {
        self.raw(number, id, cmd, res).0
    }

    fn unload(&mut self, id: u32) -> u64 {
        let rt = FakeRuntime::new(0x1000);
        let request = Lv2Request::Unsupported {
            number: UNLOAD,
            args: [u64::from(id), 0, 0, 0, 0, 0, 0, 0],
        };
        match self.host.dispatch(request, UnitId::new(0), &rt) {
            Lv2Dispatch::Immediate { code, .. } => code,
            other => panic!("sc 483: expected an immediate answer, got {other:?}"),
        }
    }

    /// liblv2.sprx's start or stop: phase 1, then a report of 0 once the
    /// kernel accepts phase 1, since neither entry CellGov hands back
    /// can run.
    fn handshake(&mut self, number: u64, id: u32) -> u64 {
        match self.rc(number, id, 1, 0) {
            0 => self.rc(number, id, 2, 0),
            refused => refused,
        }
    }

    /// CellGov's answer at `slot` is the console's.
    fn check(&self, slot: usize, cellgov: u64) {
        assert_eq!(cellgov, self.word(slot), "slot {slot}");
    }
}

#[test]
fn the_state_and_command_answers_match_the_first_state_capture() {
    let mut r = Replay::new("lv2_prx_states", 71);
    // Module A, up to the start the console refuses: see
    // `the_console_refuses_a_start_cellgov_accepts`.
    let a = r.load();
    let rc = r.rc(STOP, a, 1, 0);
    r.check(1, rc);
    let rc = r.rc(STOP, a, 4, 0);
    r.check(2, rc);
    let rc = r.rc(STOP, a, 2, 0);
    r.check(3, rc);

    for (k, cmd) in [1u64, 0x11, 0x21, 3, 0x10, 0].into_iter().enumerate() {
        let at = 17 + 5 * k;
        let b = r.load();
        let (rc, [entry, entry2]) = r.raw(START, b, cmd, 0);
        r.check(at + 1, rc);
        r.check(at + 3, entry2);
        if cmd & 0xF != 1 {
            // An unknown phase writes nothing and leaves the module
            // unloadable.
            r.check(at + 2, entry);
            let rc = r.unload(b);
            r.check(at + 4, rc);
        }
    }

    for (k, cmd) in [4u64, 0x14, 8].into_iter().enumerate() {
        let at = 47 + 8 * k;
        let c = r.load();
        let rc = r.handshake(START, c);
        r.check(at + 1, rc);
        let (rc, [entry, entry2]) = r.raw(STOP, c, cmd, 0);
        r.check(at + 2, rc);
        r.check(at + 3, entry);
        r.check(at + 4, entry2);
        let rc = r.handshake(STOP, c);
        r.check(at + 5, rc);
        let rc = r.unload(c);
        r.check(at + 6, rc);
        let rc = r.unload(c);
        r.check(at + 7, rc);
    }
}

#[test]
fn the_state_answers_match_the_second_state_capture() {
    let mut r = Replay::new("lv2_prx_states_more", 37);
    // Module A, up to the second start the console refuses.
    let a = r.load();
    let rc = r.handshake(START, a);
    r.check(1, rc);

    // Module D: a stop report before any start.
    let d = r.load();
    for (slot, number, cmd) in [(16, STOP, 2), (17, STOP, 1), (18, STOP, 4), (19, START, 2)] {
        let rc = r.rc(number, d, cmd, 0);
        r.check(slot, rc);
    }
    let rc = r.unload(d);
    r.check(20, rc);

    // Module E: a stop that cannot finish.
    let e = r.load();
    let rc = r.handshake(START, e);
    r.check(22, rc);
    for (slot, cmd, res) in [
        (23, 1, 0),
        (25, 1, 0),
        (26, 4, 0),
        (27, 2, 1),
        (28, 1, 0),
        (29, 2, 0),
    ] {
        let rc = r.rc(STOP, e, cmd, res);
        r.check(slot, rc);
    }
    let rc = r.unload(e);
    r.check(30, rc);

    // Module F: a start whose entry asks not to stay.
    let f = r.load();
    let rc = r.rc(START, f, 1, 0);
    r.check(32, rc);
    let rc = r.rc(START, f, 2, 1);
    r.check(33, rc);
    let rc = r.rc(STOP, f, 1, 0);
    r.check(34, rc);
    let rc = r.unload(f);
    r.check(35, rc);
    let rc = r.unload(f);
    r.check(36, rc);
}

#[test]
fn the_state_answers_match_the_third_state_capture() {
    let mut r = Replay::new("lv2_prx_states_last", 23);
    // Module A, up to the call after the refused unload.
    let a = r.load();
    let rc = r.handshake(START, a);
    r.check(1, rc);
    let rc = r.unload(a);
    r.check(2, rc);

    // Module B: a start report to a started module.
    let b = r.load();
    let rc = r.handshake(START, b);
    r.check(7, rc);
    for (slot, number, cmd) in [(8, START, 2), (9, STOP, 1), (10, STOP, 2)] {
        let rc = r.rc(number, b, cmd, 0);
        r.check(slot, rc);
    }
    let rc = r.unload(b);
    r.check(11, rc);

    // Module C: a start report of 5.
    let c = r.load();
    let rc = r.rc(START, c, 1, 0);
    r.check(13, rc);
    let rc = r.rc(START, c, 2, 5);
    r.check(14, rc);
    let rc = r.rc(STOP, c, 1, 0);
    r.check(15, rc);
    let rc = r.unload(c);
    r.check(16, rc);

    // Module G, up to the unload between the two start phases.
    let g = r.load();
    let rc = r.rc(START, g, 1, 0);
    r.check(18, rc);
}

/// The console tracks a start in progress and refuses a start of any
/// module that is not freshly loaded; a refused unload of a started or
/// starting module still withdraws it. CellGov's sc 480 hands back the
/// boot's own instance of a module the boot already loaded, so the
/// console's rule would refuse a game's start of that module.
/// CellGov keeps its answers until a load hands back a fresh instance.
#[test]
fn the_console_refuses_a_start_cellgov_accepts() {
    let esrch: u64 = errno::CELL_ESRCH.into();

    // A start after a stop report to a never-started module.
    let mut r = Replay::new("lv2_prx_states", 71);
    let a = r.load();
    for cmd in [1, 4, 2] {
        r.rc(STOP, a, cmd, 0);
    }
    let cellgov = r.handshake(START, a);
    assert_ne!(r.word(4), cellgov, "console refuses the start");
    assert_eq!(cellgov, 0, "CellGov starts the stopped module");

    // A second start of a started module.
    let mut r = Replay::new("lv2_prx_states_more", 37);
    let a = r.load();
    r.handshake(START, a);
    let cellgov = r.handshake(START, a);
    assert_ne!(r.word(2), cellgov, "console refuses the second start");
    assert_eq!(cellgov, 0, "CellGov starts it again");

    // A started module after its refused unload, and an unload between
    // the two start phases.
    let mut r = Replay::new("lv2_prx_states_last", 23);
    let a = r.load();
    r.handshake(START, a);
    r.unload(a);
    let cellgov = r.rc(STOP, a, 1, 0);
    assert_eq!(r.word(3), esrch, "console withdrew the module");
    assert_eq!(cellgov, 0, "CellGov keeps it");
    for _ in 0..2 {
        r.load();
    }
    let g = r.load();
    r.rc(START, g, 1, 0);
    let cellgov = r.unload(g);
    assert_ne!(r.word(19), cellgov, "console refuses the unload");
    assert_eq!(cellgov, 0, "CellGov withdraws the unstarted module");
}

/// The console's start and stop phase 1 hand back the module's own
/// `module_start` and `module_stop` entries; CellGov runs every firmware
/// module's `module_start` itself and hands back none.
#[test]
fn the_console_hands_back_entries_cellgov_does_not_run() {
    let mut r = Replay::new("lv2_prx_states", 71);
    let b = r.load();
    let (_, [entry, _]) = r.raw(START, b, 1, 0);
    assert_eq!(entry, NO_ENTRY);
    assert_ne!(r.word(19), NO_ENTRY, "console: module_start's entry");

    let mut r = Replay::new("lv2_prx_states_more", 37);
    for _ in 0..2 {
        r.load();
    }
    let e = r.load();
    r.handshake(START, e);
    let (_, [entry, _]) = r.raw(STOP, e, 1, 0);
    assert_eq!(entry, NO_ENTRY);
    assert_ne!(r.word(24), NO_ENTRY, "console: module_stop's entry");
}
