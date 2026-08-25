//! Install-progress reporting seam: the installers emit events, a
//! caller-supplied sink renders them (or `()` drops them). Nothing in
//! this module or its callers knows what a terminal is.

/// A coarse stage of an install, for a reporter to label its output.
///
/// `Staging` is the only stage with a byte denominator; the others
/// have no natural progress unit and a renderer should treat them as
/// indeterminate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Reading and validating the container's structure.
    Reading,
    /// Writing the game tree into the staging root.
    Staging,
    /// Running the decrypt-proof gate.
    Proving,
    /// Hashing the source container for the install record.
    Hashing,
    /// Clearing an existing target directory (`force` overwrite).
    Clearing,
    /// The commit rename sequence.
    Committing,
}

/// Install progress sink.
///
/// Implementations must be cheap: [`Self::bytes_advanced`] is called
/// from the staging write loop once per piece, so it should amount to
/// an atomic add. `Sync` because a renderer reads the sink's state
/// from its own thread while the installer writes.
pub trait InstallProgress: Sync {
    /// The install entered `phase`.
    fn phase(&self, phase: Phase);
    /// Total staging work, known before the first write. `files`
    /// counts staged entries, which can exceed the deduplicated
    /// record count.
    fn totals(&self, files: usize, bytes: u64);
    /// Staging began writing the file at `path` (`/`-separated).
    fn file_started(&self, path: &str);
    /// Bytes newly written since the last call.
    fn bytes_advanced(&self, delta: u64);
    /// The current file finished (written, hashed, synced).
    fn file_finished(&self);
    /// The install completed successfully.
    fn finished(&self);
}

/// The no-op reporter: an install with `&()` behaves exactly as it
/// did before instrumentation.
impl InstallProgress for () {
    fn phase(&self, _phase: Phase) {}
    fn totals(&self, _files: usize, _bytes: u64) {}
    fn file_started(&self, _path: &str) {}
    fn bytes_advanced(&self, _delta: u64) {}
    fn file_finished(&self) {}
    fn finished(&self) {}
}
