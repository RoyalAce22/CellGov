//! `WatchTaps` hands the boot its observers, the store watch stamps each
//! write with the PC the PPU half saw last, and what the watches report
//! reaches the reporter.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Write;
use std::rc::Rc;

use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::state::PpuState;

use super::*;
use crate::taps::store_watch::{pack_record, RECORD_LEN};
use crate::taps::{
    HleWatch, HleWatchSpec, RecordFile, StoreWatch, StoreWatchSpec, ValueSample, ValueSampleSpec,
};

/// Every event, rendered, in the order it arrived.
fn collecting() -> (WatchReporter, Rc<RefCell<Vec<String>>>) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&seen);
    let report: WatchReporter = Rc::new(move |event: WatchEvent<'_>| {
        sink.borrow_mut().push(match event {
            WatchEvent::Bound(line) => format!("bound {line}"),
            WatchEvent::WriteFailed { watch, error } => format!("failed {watch:?} {error}"),
        });
    });
    (report, seen)
}

/// A writer that takes the header and refuses every write after it.
struct HeaderOnly(usize);

impl Write for HeaderOnly {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.0 > 0 {
            self.0 = 0;
            return Ok(buf.len());
        }
        Err(std::io::Error::other("disk full"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_store_write_carries_the_last_dispatched_pc() {
    let dir = cellgov_testkit::scratch::scratch_labeled("taps_watch_store");
    let path = dir.join("store.bin");
    let spec = StoreWatchSpec {
        addr: 0x1000,
        len: 0x10,
        path: path.clone(),
    };
    let store = StoreWatch::new(&spec, RecordFile::create(&path, &spec.header()).unwrap());
    let (report, seen) = collecting();
    let taps = WatchTaps::new(None, Some(store), None, report);
    let ppu = taps.ppu().expect("the store watch needs the PPU half");
    let mut runtime = taps.runtime().expect("runtime half");
    assert!(
        taps.runtime().is_none(),
        "the runtime half is handed out once"
    );

    let mut state = PpuState::new();
    state.pc = 0x0001_2340;
    ppu.dispatch(UnitId::new(0), &PpuInstruction::Consumed, &state);
    runtime.write(0, 0x1004, &[0xEE; 4]);
    runtime.write(1, 0x1004, &[0xEE; 4]);
    drop(runtime);

    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[0..4], b"CGSW");
    assert_eq!(
        bytes.len(),
        16 + RECORD_LEN,
        "a write in another space is not this watch's"
    );
    assert_eq!(
        &bytes[16..],
        &pack_record(
            0,
            0x0001_2340,
            0x1004,
            4,
            u64::from_le_bytes([0xEE, 0xEE, 0xEE, 0xEE, 0, 0, 0, 0])
        )
    );
    assert!(seen.borrow().is_empty(), "{:?}", seen.borrow());
}

#[test]
fn a_value_sample_alone_installs_no_ppu_observer() {
    let spec = ValueSampleSpec {
        addr: 0x100,
        width: 4,
        stride: 1,
        path: "s".into(),
    };
    let sample = ValueSample::new(&spec, RecordFile::over(Vec::new(), &[]).unwrap());
    let (report, _) = collecting();
    let taps = WatchTaps::new(None, None, Some(sample), report);
    assert!(taps.ppu().is_none());
    assert!(taps.runtime().is_some());
}

#[test]
fn each_watchs_first_write_failure_is_reported_once() {
    let store_spec = StoreWatchSpec {
        addr: 0x1000,
        len: 0x10,
        path: "w".into(),
    };
    let sample_spec = ValueSampleSpec {
        addr: 0x100,
        width: 4,
        stride: 1,
        path: "s".into(),
    };
    let store = StoreWatch::new(&store_spec, RecordFile::over(HeaderOnly(1), b"H").unwrap());
    let sample = ValueSample::new(&sample_spec, RecordFile::over(HeaderOnly(1), b"H").unwrap());
    let (report, seen) = collecting();
    let taps = WatchTaps::new(None, Some(store), Some(sample), report);
    let mut runtime = taps.runtime().unwrap();
    let mem = GuestMemory::new(0x1000);
    for step in 1..=3 {
        runtime.write(0, 0x1000, &[1]);
        runtime.step(step, &mem);
    }
    assert_eq!(
        *seen.borrow(),
        ["failed Store disk full", "failed ValueSample disk full"]
    );
}

#[test]
fn binding_reports_each_line_after_a_resolution_write_failure() {
    let spec = HleWatchSpec {
        nids: vec![0xAA, 0xBB],
        raw_pcs: Vec::new(),
        path: "w".into(),
    };
    let hle = HleWatch::new(&spec, RecordFile::over(HeaderOnly(1), b"H").unwrap());
    let (report, seen) = collecting();
    let taps = WatchTaps::new(Some(hle), None, None, report);

    let mut mem = GuestMemory::new(0x1000);
    let opd = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0x100), 4).unwrap();
    mem.apply_commit(opd, &0x0001_0000u32.to_be_bytes())
        .unwrap();
    let exports = BTreeMap::from([("libA".to_string(), BTreeMap::from([(0xAA, 0x100)]))]);
    taps.firmware_bound(1, &exports, &mem);
    assert!(seen.borrow().is_empty(), "a child's set binds nothing here");
    taps.firmware_bound(0, &exports, &mem);

    let seen = seen.borrow();
    assert_eq!(seen.len(), 3, "{seen:?}");
    assert_eq!(seen[0], "failed HleReturn disk full");
    assert!(
        seen[1].starts_with("bound resolved NID 0x000000aa"),
        "{seen:?}"
    );
    assert_eq!(
        seen[2],
        "bound NID 0x000000bb not present in firmware export table"
    );
}

/// A writer that takes `.0` bytes in all and refuses every write past them.
struct Budget(usize);

impl Write for Budget {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.len() > self.0 {
            return Err(std::io::Error::other("disk full"));
        }
        self.0 -= buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_dispatch_reports_the_hle_watchs_first_write_failure_once() {
    let spec = HleWatchSpec {
        nids: Vec::new(),
        raw_pcs: vec![(0x1_0000, "f".to_string())],
        path: "w".into(),
    };
    // The 1-byte header and the 11-byte resolution record fit; the
    // entry record does not.
    let mut hle = HleWatch::new(&spec, RecordFile::over(Budget(1 + 11), b"H").unwrap());
    assert!(hle.take_write_failure().is_none(), "the resolution fits");
    let (report, seen) = collecting();
    let taps = WatchTaps::new(Some(hle), None, None, report);
    let ppu = taps.ppu().expect("the HLE watch needs the PPU half");

    let mut state = PpuState::new();
    state.pc = 0x1_0000;
    state.set_lr(0x2_0004);
    ppu.dispatch(UnitId::new(0), &PpuInstruction::Consumed, &state);
    state.pc = 0x2_0004;
    ppu.dispatch(UnitId::new(0), &PpuInstruction::Consumed, &state);

    assert_eq!(*seen.borrow(), ["failed HleReturn disk full"]);
}
