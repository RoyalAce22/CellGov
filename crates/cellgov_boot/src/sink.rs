//! Where a boot's narration leaves the library.
//!
//! Nothing below this module writes to a console. A boot narrates
//! through a [`BootSink`] the caller supplies, and the caller decides
//! where each channel lands.

/// Where a boot writes its narration.
///
/// The three channels are separate because the caller routes them
/// differently.
///
/// Cross-module contract: [`Self::note`] and [`Self::warn`] receive a
/// line with no terminator, and the implementation supplies one;
/// [`Self::guest_text`] receives an already-decoded fragment to pass
/// through unchanged, which may end mid-line.
pub trait BootSink {
    /// The boot's own account of a stage it completed.
    fn note(&self, line: &str);

    /// An anomaly the boot survived.
    fn warn(&self, line: &str);

    /// Text the guest emitted.
    fn guest_text(&self, text: &str);
}

/// A [`BootSink`] that drops every channel.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullSink;

impl BootSink for NullSink {
    fn note(&self, _line: &str) {}
    fn warn(&self, _line: &str) {}
    fn guest_text(&self, _text: &str) {}
}
