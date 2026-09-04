//! What instrumented code emits progress against.

/// Where instrumented code reports its progress.
///
/// [`Self::advanced`] is called from a write or step loop once per
/// piece, so an implementation should amount to an atomic add.
///
/// Phase codes index the [`super::Task`]'s label table; the sink never
/// sees the labels.
pub trait ProgressSink: Sync {
    /// The work entered the phase at `code`.
    fn phase(&self, code: u8);
    /// Total work, known before the first advance. `items` counts the
    /// secondary unit (staged files, queued downloads), which can
    /// exceed what finally lands; `amount` is the denominator in the
    /// task's unit.
    fn totals(&self, items: usize, amount: u64);
    /// Work already done before this run started, for a resumed
    /// transfer; it counts toward the ratio but not the rate. Call
    /// before the first [`Self::advanced`].
    fn preset_done(&self, amount: u64);
    /// Work began on the item named `name`.
    fn item_started(&self, name: &str);
    /// Amount newly done since the last call, in the task's unit.
    fn advanced(&self, delta: u64);
    /// The current item finished.
    fn item_finished(&self);
    /// The work completed successfully.
    ///
    /// A live bar stops drawing within one render tick of this call.
    /// The stop is not a rendezvous. To keep later output off the
    /// bar's last frame, wait for the render thread:
    ///
    /// - [`super::ProgressBar::finish`] joins the thread.
    /// - [`super::release_terminal`] stops the thread on an exit path
    ///   that runs no destructor.
    fn finished(&self);
}

/// The no-op sink.
impl ProgressSink for () {
    fn phase(&self, _code: u8) {}
    fn totals(&self, _items: usize, _amount: u64) {}
    fn preset_done(&self, _amount: u64) {}
    fn item_started(&self, _name: &str) {}
    fn advanced(&self, _delta: u64) {}
    fn item_finished(&self) {}
    fn finished(&self) {}
}
