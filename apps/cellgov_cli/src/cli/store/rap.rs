//! Resolves the RAP a decrypt needs, from `--rap` or from exdata.

use std::path::Path;

use cellgov_install::npdrm::{read_rap, Rap, RapPresence, RapReadError};

use super::StoreCliError;

/// Resolve the RAP for one NPDRM content id: from `explicit` when
/// `--rap` named a file, otherwise from `<exdata>/<id>.rap`.
///
/// # Errors
///
/// [`StoreCliError::Rap`] for a file that is there and unreadable or
/// not 16 bytes, and [`StoreCliError::ExplicitRapMissing`] when `--rap`
/// named a file that is not there.
///
/// Only the exdata probe may miss quietly. An explicit flag that
/// resolved to nothing would look the same as not passing it. License-3
/// titles decrypt on the free-key fallback either way.
pub(crate) fn resolve(
    explicit: Option<&Path>,
    exdata: &Path,
    content_id: &str,
) -> Result<Option<Rap>, StoreCliError> {
    match explicit {
        Some(path) => read_rap(path, RapPresence::Required).map_err(|e| match e {
            RapReadError::Missing { path } => StoreCliError::ExplicitRapMissing { path },
            e => StoreCliError::Rap(e),
        }),
        None => read_rap(
            &exdata.join(format!("{content_id}.rap")),
            RapPresence::MayBeAbsent,
        )
        .map_err(StoreCliError::Rap),
    }
}

#[cfg(test)]
#[path = "tests/rap_tests.rs"]
mod tests;
