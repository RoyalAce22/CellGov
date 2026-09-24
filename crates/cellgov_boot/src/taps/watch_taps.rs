//! The watches one run installs, as the observers the boot asks for.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::io::Write;
use std::rc::Rc;

use cellgov_core::RuntimeTap;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::state::PpuState;
use cellgov_ppu::PpuTap;

use super::hle_watch::HleWatch;
use super::store_watch::StoreWatch;
use super::value_sample::ValueSample;
use super::DebugTaps;

/// Which watch an event came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchKind {
    /// The HLE return watch.
    HleReturn,
    /// The store watch.
    Store,
    /// The value sample.
    ValueSample,
}

/// What a running watch reports to the program that installed it.
#[derive(Debug)]
pub enum WatchEvent<'a> {
    /// One line of binding the HLE watch's NIDs against a firmware set:
    /// a NID resolved, missing, exported by several libraries, behind an
    /// unmapped OPD, or kept at an earlier set's entry.
    Bound(&'a str),
    /// A watch's capture refused a record. The capture ends there; the
    /// run goes on.
    WriteFailed {
        /// The watch whose capture failed.
        watch: WatchKind,
        /// The host's refusal.
        error: &'a std::io::Error,
    },
}

/// Where a running watch's events go.
pub type WatchReporter = Rc<dyn Fn(WatchEvent<'_>)>;

/// The PPU half: the HLE return watch, and the last dispatched PC the
/// store watch stamps its records with.
struct PpuTaps<W: Write> {
    hle: Option<RefCell<HleWatch<W>>>,
    last_pc: Option<Rc<Cell<u32>>>,
    report: WatchReporter,
}

impl<W: Write> PpuTap for PpuTaps<W> {
    fn dispatch(&self, unit: UnitId, insn: &PpuInstruction, state: &PpuState) {
        if let Some(last_pc) = &self.last_pc {
            last_pc.set(state.pc as u32);
        }
        if let Some(hle) = &self.hle {
            let mut hle = hle.borrow_mut();
            hle.dispatch(unit, insn, state);
            if let Some(error) = hle.take_write_failure() {
                (self.report)(WatchEvent::WriteFailed {
                    watch: WatchKind::HleReturn,
                    error: &error,
                });
            }
        }
    }
}

/// The runtime half: the store watch and the value sample.
struct RuntimeTaps<W: Write> {
    store: Option<StoreWatch<W>>,
    sample: Option<ValueSample<W>>,
    last_pc: Rc<Cell<u32>>,
    report: WatchReporter,
}

impl<W: Write> RuntimeTap for RuntimeTaps<W> {
    fn write(&mut self, space: u32, addr: u64, bytes: &[u8]) {
        if space == 0 {
            if let Some(store) = &mut self.store {
                store.write(self.last_pc.get(), addr, bytes);
                if let Some(error) = store.take_write_failure() {
                    (self.report)(WatchEvent::WriteFailed {
                        watch: WatchKind::Store,
                        error: &error,
                    });
                }
            }
        }
    }

    fn step(&mut self, step: u64, memory: &GuestMemory) {
        if let Some(sample) = &mut self.sample {
            sample.step(step, memory);
            if let Some(error) = sample.take_write_failure() {
                (self.report)(WatchEvent::WriteFailed {
                    watch: WatchKind::ValueSample,
                    error: &error,
                });
            }
        }
    }
}

/// The watches one run installs.
///
/// The store watch stamps each record with the PC the PPU half saw
/// last, so it installs both halves. A value sample alone installs no
/// PPU observer, and an HLE watch alone no runtime observer.
pub struct WatchTaps<W: Write> {
    ppu: Option<Rc<PpuTaps<W>>>,
    /// The runtime half, until the boot's one [`DebugTaps::runtime`]
    /// call takes it.
    runtime: RefCell<Option<RuntimeTaps<W>>>,
    report: WatchReporter,
}

impl<W: Write> WatchTaps<W> {
    /// Install the watches given, each already writing to its capture.
    /// `report` receives what they report while the run goes.
    pub fn new(
        hle: Option<HleWatch<W>>,
        store: Option<StoreWatch<W>>,
        sample: Option<ValueSample<W>>,
        report: WatchReporter,
    ) -> Self {
        let last_pc = Rc::new(Cell::new(0));
        let ppu = (hle.is_some() || store.is_some()).then(|| {
            Rc::new(PpuTaps {
                hle: hle.map(RefCell::new),
                last_pc: store.is_some().then(|| Rc::clone(&last_pc)),
                report: Rc::clone(&report),
            })
        });
        let runtime = (store.is_some() || sample.is_some()).then(|| RuntimeTaps {
            store,
            sample,
            last_pc,
            report: Rc::clone(&report),
        });
        Self {
            ppu,
            runtime: RefCell::new(runtime),
            report,
        }
    }
}

/// The code address the OPD at `opd` holds.
fn opd_code(mem: &GuestMemory, opd: u32) -> Option<u32> {
    let range = ByteRange::new(GuestAddr::new(u64::from(opd)), 4)?;
    let bytes = mem.read(range)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

impl<W: Write + 'static> DebugTaps for WatchTaps<W> {
    fn ppu(&self) -> Option<Rc<dyn PpuTap>> {
        self.ppu.as_ref().map(|p| Rc::clone(p) as Rc<dyn PpuTap>)
    }

    fn runtime(&self) -> Option<Box<dyn RuntimeTap>> {
        self.runtime
            .borrow_mut()
            .take()
            .map(|r| Box::new(r) as Box<dyn RuntimeTap>)
    }

    fn firmware_bound(
        &self,
        space: u32,
        exports: &BTreeMap<String, BTreeMap<u32, u32>>,
        mem: &GuestMemory,
    ) {
        if space != 0 {
            return;
        }
        let Some(hle) = self.ppu.as_ref().and_then(|p| p.hle.as_ref()) else {
            return;
        };
        let mut hle = hle.borrow_mut();
        let lines = hle.bind(exports, |opd| opd_code(mem, opd));
        // A resolution record is written while the set binds, before
        // the lines that report it.
        if let Some(error) = hle.take_write_failure() {
            (self.report)(WatchEvent::WriteFailed {
                watch: WatchKind::HleReturn,
                error: &error,
            });
        }
        for line in &lines {
            (self.report)(WatchEvent::Bound(line));
        }
    }
}

#[cfg(test)]
#[path = "tests/watch_taps_tests.rs"]
mod tests;
