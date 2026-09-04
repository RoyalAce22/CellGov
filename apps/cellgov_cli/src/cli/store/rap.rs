//! Resolves the RAP a decrypt needs, from `--rap` or from exdata.

use std::path::Path;

use cellgov_install::npdrm::Rap;

use super::StoreCliError;

/// Read a 16-byte RAP file.
///
/// # Errors
///
/// - [`StoreCliError::RapWrongSize`] for a file that is not exactly 16
///   bytes.
/// - [`StoreCliError::RapReadFailed`] for a file that is there but
///   unreadable.
///
/// Only absence yields `Ok(None)`. That is the ordinary "not installed"
/// case, which the NPDRM layer turns into the license-3 free-key
/// fallback or a named refusal. Any other read failure would look the
/// same as absence and slip through as the free key.
pub(crate) fn from_file(path: &Path) -> Result<Option<Rap>, StoreCliError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(StoreCliError::RapReadFailed {
                path: path.to_path_buf(),
                source,
            })
        }
    };
    let arr: [u8; 16] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| StoreCliError::RapWrongSize {
            path: path.to_path_buf(),
            len: bytes.len(),
        })?;
    Ok(Some(Rap(arr)))
}

/// Resolve the RAP for one NPDRM content id: from `explicit` when
/// `--rap` named a file, otherwise from `<exdata>/<id>.rap`.
///
/// # Errors
///
/// Everything [`from_file`] refuses, plus
/// [`StoreCliError::ExplicitRapMissing`] when `--rap` named a file that
/// is not there.
///
/// Only the exdata probe may miss quietly. An explicit flag that
/// resolved to nothing would look the same as not passing it. License-3
/// titles decrypt on the free-key fallback either way.
pub(crate) fn resolve(
    explicit: Option<&Path>,
    exdata: &Path,
    content_id: &str,
) -> Result<Option<Rap>, StoreCliError> {
    let rap = match explicit {
        Some(p) => p.to_path_buf(),
        None => exdata.join(format!("{content_id}.rap")),
    };
    match from_file(&rap)? {
        Some(k) => Ok(Some(k)),
        None if explicit.is_some() => Err(StoreCliError::ExplicitRapMissing { path: rap }),
        None => Ok(None),
    }
}

#[cfg(test)]
#[path = "tests/rap_tests.rs"]
mod tests;
