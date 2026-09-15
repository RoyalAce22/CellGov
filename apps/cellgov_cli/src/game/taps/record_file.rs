//! The capture file a watch appends its records to.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use super::error::TapError;

/// A capture file that a watch appends records to.
///
/// [`Self::append`] flushes each record, so a run that stops mid-way
/// leaves the file readable up to its last whole record. The first
/// write that fails ends the capture, and `append` reports it. A failed
/// write can leave part of a record in the file. A reader walks the
/// records from the header, so every later record then sits at the
/// wrong offset.
pub(super) struct RecordFile<W: Write> {
    out: W,
    label: &'static str,
    failed: bool,
}

impl RecordFile<BufWriter<File>> {
    /// Create the file at `path` and write `header` to it.
    ///
    /// # Errors
    ///
    /// [`TapError::Capture`] when the host refuses the create or the
    /// header write.
    pub(super) fn create(
        label: &'static str,
        path: &Path,
        header: &[u8],
    ) -> Result<Self, TapError> {
        File::create(path)
            .and_then(|file| Self::over(label, BufWriter::new(file), header))
            .map_err(|source| TapError::Capture {
                label,
                path: path.to_path_buf(),
                source,
            })
    }
}

impl<W: Write> RecordFile<W> {
    /// Write `header` to `out` and append records after it.
    ///
    /// # Errors
    ///
    /// `out` refuses the header.
    pub(super) fn over(label: &'static str, mut out: W, header: &[u8]) -> std::io::Result<Self> {
        out.write_all(header)?;
        out.flush()?;
        Ok(Self {
            out,
            label,
            failed: false,
        })
    }

    /// Append one whole record; after the first failed write, do nothing.
    pub(super) fn append(&mut self, record: &[u8]) {
        if self.failed {
            return;
        }
        if let Err(e) = self.out.write_all(record).and_then(|()| self.out.flush()) {
            self.failed = true;
            eprintln!(
                "[cellgov] {}: write failed: {e}; the capture is truncated from here",
                self.label
            );
        }
    }

    /// The writer the records went to.
    #[cfg(test)]
    pub(super) fn into_inner(self) -> W {
        self.out
    }
}
