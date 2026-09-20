//! The watches the environment asks for, as the observers a boot
//! installs.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use cellgov_boot::{DebugTaps, NoTaps};
use cellgov_core::RuntimeTap;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::state::PpuState;
use cellgov_ppu::PpuTap;

use crate::env_vars;

use super::error::TapError;
use super::hle_watch::{HleWatch, HleWatchSpec};
use super::record_file::RecordFile;
use super::store_watch::{StoreWatch, StoreWatchSpec};
use super::value_sample::{ValueSample, ValueSampleSpec};

type Capture = BufWriter<File>;

/// The value of `var`, `None` when unset.
fn var(var: &'static str) -> Result<Option<String>, TapError> {
    match std::env::var(var) {
        Ok(v) => Ok(Some(v)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(TapError::NotUnicode { var }),
    }
}

/// Every variable a watch reads.
const WATCH_VARS: [&str; 8] = [
    env_vars::HLE_RETURN_WATCH,
    env_vars::HLE_RETURN_WATCH_PCS,
    env_vars::HLE_RETURN_WATCH_PATH,
    env_vars::STORE_WATCH,
    env_vars::STORE_WATCH_PATH,
    env_vars::VALUE_SAMPLE,
    env_vars::VALUE_SAMPLE_PATH,
    env_vars::VALUE_SAMPLE_STRIDE,
];

/// The watch variables set to a non-empty value, for a command that
/// installs no watch to name what it ignores.
pub(crate) fn set_watch_vars() -> Vec<&'static str> {
    WATCH_VARS
        .into_iter()
        .filter(|v| std::env::var_os(v).is_some_and(|x| !x.is_empty()))
        .collect()
}

/// The observers the `CELLGOV_HLE_RETURN_WATCH*`,
/// `CELLGOV_STORE_WATCH*` and `CELLGOV_VALUE_SAMPLE*` variables ask
/// for, with every capture file created and its header written.
///
/// # Errors
///
/// Returns a [`TapError`] when:
///
/// - a variable does not parse or is out of range
/// - one variable of a pair is set without the other
/// - two watches name one capture file
/// - the host refuses to create a capture or to write its header
pub(crate) fn from_env() -> Result<Rc<dyn DebugTaps>, TapError> {
    let hle = HleWatchSpec::parse(
        var(env_vars::HLE_RETURN_WATCH)?.as_deref(),
        var(env_vars::HLE_RETURN_WATCH_PCS)?.as_deref(),
        var(env_vars::HLE_RETURN_WATCH_PATH)?.as_deref(),
    )?;
    let store = StoreWatchSpec::parse(
        var(env_vars::STORE_WATCH)?.as_deref(),
        var(env_vars::STORE_WATCH_PATH)?.as_deref(),
    )?;
    let sample = ValueSampleSpec::parse(
        var(env_vars::VALUE_SAMPLE)?.as_deref(),
        var(env_vars::VALUE_SAMPLE_PATH)?.as_deref(),
        var(env_vars::VALUE_SAMPLE_STRIDE)?.as_deref(),
    )?;
    if hle.is_none() && store.is_none() && sample.is_none() {
        return Ok(Rc::new(NoTaps));
    }
    Ok(Rc::new(EnvTaps::open(hle, store, sample)?))
}

/// The PPU half: the HLE return watch, and the last dispatched PC the
/// store watch stamps its records with.
struct PpuTaps {
    hle: Option<RefCell<HleWatch<Capture>>>,
    last_pc: Option<Rc<Cell<u32>>>,
}

impl PpuTap for PpuTaps {
    fn dispatch(&self, unit: UnitId, insn: &PpuInstruction, state: &PpuState) {
        if let Some(last_pc) = &self.last_pc {
            last_pc.set(state.pc as u32);
        }
        if let Some(hle) = &self.hle {
            hle.borrow_mut().dispatch(unit, insn, state);
        }
    }
}

/// The runtime half: the store watch and the value sample.
struct RuntimeTaps {
    store: Option<StoreWatch<Capture>>,
    sample: Option<ValueSample<Capture>>,
    last_pc: Rc<Cell<u32>>,
}

impl RuntimeTap for RuntimeTaps {
    fn write(&mut self, space: u32, addr: u64, bytes: &[u8]) {
        if space == 0 {
            if let Some(store) = &mut self.store {
                store.write(self.last_pc.get(), addr, bytes);
            }
        }
    }

    fn step(&mut self, step: u64, memory: &GuestMemory) {
        if let Some(sample) = &mut self.sample {
            sample.step(step, memory);
        }
    }
}

/// The watches one run installs.
struct EnvTaps {
    ppu: Option<Rc<PpuTaps>>,
    /// The runtime half, until the boot's one [`DebugTaps::runtime`] call takes it.
    runtime: RefCell<Option<RuntimeTaps>>,
}

const HLE_LABEL: &str = "hle-return-watch";
const STORE_LABEL: &str = "store-watch";
const SAMPLE_LABEL: &str = "value-sample";

/// Refuse two watches that name one capture file.
///
/// Each watch creates its file with its own handle, so a shared path
/// holds one watch's header over the other's records.
fn refuse_shared_paths(paths: &[(&'static str, &Path)]) -> Result<(), TapError> {
    let resolved: Vec<PathBuf> = paths
        .iter()
        .map(|(_, p)| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()))
        .collect();
    for (i, &(first, _)) in paths.iter().enumerate() {
        for (j, &(second, _)) in paths.iter().enumerate().skip(i + 1) {
            if resolved[i] == resolved[j] {
                return Err(TapError::SharedCapture {
                    first,
                    second,
                    path: paths[j].1.to_path_buf(),
                });
            }
        }
    }
    Ok(())
}

impl EnvTaps {
    /// Create every capture the specs name and report each watch.
    fn open(
        hle: Option<HleWatchSpec>,
        store: Option<StoreWatchSpec>,
        sample: Option<ValueSampleSpec>,
    ) -> Result<Self, TapError> {
        let paths: Vec<(&'static str, &Path)> = [
            hle.as_ref().map(|s| (HLE_LABEL, s.path.as_path())),
            store.as_ref().map(|s| (STORE_LABEL, s.path.as_path())),
            sample.as_ref().map(|s| (SAMPLE_LABEL, s.path.as_path())),
        ]
        .into_iter()
        .flatten()
        .collect();
        refuse_shared_paths(&paths)?;
        let hle = match hle {
            Some(spec) => {
                let out = RecordFile::create(HLE_LABEL, &spec.path, &spec.header())?;
                eprintln!(
                    "[cellgov] hle-return-watch active: {} NID(s), {} raw PC(s), path={}",
                    spec.nids.len(),
                    spec.raw_pcs.len(),
                    spec.path.display()
                );
                Some(RefCell::new(HleWatch::new(&spec, out)))
            }
            None => None,
        };
        let store = match store {
            Some(spec) => {
                let out = RecordFile::create(STORE_LABEL, &spec.path, &spec.header())?;
                eprintln!(
                    "[cellgov] store-watch active: addr=0x{:x} len=0x{:x} path={}",
                    spec.addr,
                    spec.len,
                    spec.path.display()
                );
                Some(StoreWatch::new(&spec, out))
            }
            None => None,
        };
        let sample = match sample {
            Some(spec) => {
                let out = RecordFile::create(SAMPLE_LABEL, &spec.path, &spec.header())?;
                eprintln!(
                    "[cellgov] value-sample active: addr=0x{:x} width={} stride={} path={}",
                    spec.addr,
                    spec.width,
                    spec.stride,
                    spec.path.display()
                );
                Some(ValueSample::new(&spec, out))
            }
            None => None,
        };
        let last_pc = Rc::new(Cell::new(0));
        let ppu = (hle.is_some() || store.is_some()).then(|| {
            Rc::new(PpuTaps {
                hle,
                last_pc: store.is_some().then(|| Rc::clone(&last_pc)),
            })
        });
        let runtime = (store.is_some() || sample.is_some()).then(|| RuntimeTaps {
            store,
            sample,
            last_pc,
        });
        Ok(Self {
            ppu,
            runtime: RefCell::new(runtime),
        })
    }
}

/// The code address the OPD at `opd` holds.
fn opd_code(mem: &GuestMemory, opd: u32) -> Option<u32> {
    let range = ByteRange::new(GuestAddr::new(u64::from(opd)), 4)?;
    let bytes = mem.read(range)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

impl DebugTaps for EnvTaps {
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
        for line in hle.borrow_mut().bind(exports, |opd| opd_code(mem, opd)) {
            eprintln!("[cellgov] hle-return-watch: {line}");
        }
    }
}

#[cfg(test)]
#[path = "tests/bundle_tests.rs"]
mod tests;
