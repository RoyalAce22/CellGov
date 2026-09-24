//! The capture file a watch appends its records to.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

/// A capture file that a watch appends records to.
///
/// [`Self::append`] flushes each record, so a run that stops mid-way
/// leaves the file readable up to its last whole record. The first
/// write that fails ends the capture, and `append` returns that failure
/// once. A failed write can leave part of a record in the file. A
/// reader walks the records from the header, so every later record
/// would then sit at the wrong offset.
pub struct RecordFile<W: Write> {
    out: W,
    failed: bool,
}

impl RecordFile<BufWriter<File>> {
    /// Create the file at `path` and write `header` to it.
    ///
    /// # Errors
    ///
    /// The host refuses the create or the header write.
    pub fn create(path: &Path, header: &[u8]) -> std::io::Result<Self> {
        File::create(path).and_then(|file| Self::over(BufWriter::new(file), header))
    }
}

impl<W: Write> RecordFile<W> {
    /// Write `header` to `out` and append records after it.
    ///
    /// # Errors
    ///
    /// `out` refuses the header.
    pub fn over(mut out: W, header: &[u8]) -> std::io::Result<Self> {
        out.write_all(header)?;
        out.flush()?;
        Ok(Self { out, failed: false })
    }

    /// Append one whole record; after the first failed write, do nothing.
    ///
    /// # Errors
    ///
    /// The write that ends the capture. Every later call is `Ok` and
    /// writes nothing.
    pub fn append(&mut self, record: &[u8]) -> std::io::Result<()> {
        if self.failed {
            return Ok(());
        }
        self.out
            .write_all(record)
            .and_then(|()| self.out.flush())
            .inspect_err(|_| self.failed = true)
    }

    /// The writer the records went to.
    #[cfg(test)]
    pub(crate) fn into_inner(self) -> W {
        self.out
    }
}

/// The first write failure a watch met, held until its owner takes it.
#[derive(Debug, Default)]
pub(crate) struct FirstFailure(Option<std::io::Error>);

impl FirstFailure {
    /// Keep `result`'s error when none is held yet.
    pub(crate) fn note(&mut self, result: std::io::Result<()>) {
        if let Err(e) = result {
            self.0.get_or_insert(e);
        }
    }

    /// The held failure, once.
    pub(crate) fn take(&mut self) -> Option<std::io::Error> {
        self.0.take()
    }
}

#[cfg(test)]
#[path = "tests/record_file_tests.rs"]
mod tests;
