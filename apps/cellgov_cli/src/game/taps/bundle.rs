//! The watches the environment asks for, as the observers a boot
//! installs.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use cellgov_boot::taps::{
    HleWatch, HleWatchSpec, RecordFile, StoreWatch, StoreWatchSpec, ValueSample, ValueSampleSpec,
    WatchEvent, WatchKind, WatchTaps,
};
use cellgov_boot::{DebugTaps, NoTaps};

use crate::env_vars;

use super::error::TapError;
use super::specs::{parse_hle, parse_sample, parse_store};

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
    let hle = parse_hle(
        var(env_vars::HLE_RETURN_WATCH)?.as_deref(),
        var(env_vars::HLE_RETURN_WATCH_PCS)?.as_deref(),
        var(env_vars::HLE_RETURN_WATCH_PATH)?.as_deref(),
    )?;
    let store = parse_store(
        var(env_vars::STORE_WATCH)?.as_deref(),
        var(env_vars::STORE_WATCH_PATH)?.as_deref(),
    )?;
    let sample = parse_sample(
        var(env_vars::VALUE_SAMPLE)?.as_deref(),
        var(env_vars::VALUE_SAMPLE_PATH)?.as_deref(),
        var(env_vars::VALUE_SAMPLE_STRIDE)?.as_deref(),
    )?;
    if hle.is_none() && store.is_none() && sample.is_none() {
        return Ok(Rc::new(NoTaps));
    }
    Ok(Rc::new(open(hle, store, sample)?))
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

/// The label a watch's stderr lines carry.
fn label(watch: WatchKind) -> &'static str {
    match watch {
        WatchKind::HleReturn => HLE_LABEL,
        WatchKind::Store => STORE_LABEL,
        WatchKind::ValueSample => SAMPLE_LABEL,
    }
}

/// Print what a running watch reports.
fn report(event: WatchEvent<'_>) {
    match event {
        WatchEvent::Bound(line) => eprintln!("[cellgov] {HLE_LABEL}: {line}"),
        WatchEvent::WriteFailed { watch, error } => eprintln!(
            "[cellgov] {}: write failed: {error}; the capture is truncated from here",
            label(watch)
        ),
    }
}

/// Create the capture at `path` and write its header.
fn create(
    label: &'static str,
    path: &Path,
    header: &[u8],
) -> Result<RecordFile<Capture>, TapError> {
    RecordFile::create(path, header).map_err(|source| TapError::Capture {
        label,
        path: path.to_path_buf(),
        source,
    })
}

/// Create every capture the specs name and report each watch.
fn open(
    hle: Option<HleWatchSpec>,
    store: Option<StoreWatchSpec>,
    sample: Option<ValueSampleSpec>,
) -> Result<WatchTaps<Capture>, TapError> {
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
            let out = create(HLE_LABEL, &spec.path, &spec.header())?;
            eprintln!(
                "[cellgov] hle-return-watch active: {} NID(s), {} raw PC(s), path={}",
                spec.nids.len(),
                spec.raw_pcs.len(),
                spec.path.display()
            );
            let mut watch = HleWatch::new(&spec, out);
            if let Some(error) = watch.take_write_failure() {
                report(WatchEvent::WriteFailed {
                    watch: WatchKind::HleReturn,
                    error: &error,
                });
            }
            Some(watch)
        }
        None => None,
    };
    let store = match store {
        Some(spec) => {
            let out = create(STORE_LABEL, &spec.path, &spec.header())?;
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
            let out = create(SAMPLE_LABEL, &spec.path, &spec.header())?;
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
    Ok(WatchTaps::new(hle, store, sample, Rc::new(report)))
}

#[cfg(test)]
#[path = "tests/bundle_tests.rs"]
mod tests;
