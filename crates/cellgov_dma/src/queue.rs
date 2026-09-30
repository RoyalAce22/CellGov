//! Deterministic priority queue of [`DmaCompletion`]s.
//!
//! Entries are keyed by `(completion_time, queue-assigned sequence)`,
//! giving a total order that preserves enqueue order among equal times.
//! `Effect::DmaEnqueue` flows through the commit pipeline into this
//! queue; completions emit wake events as they drain.
//!
//! The queue is every SPU's MFC command queue at once: an issuer's
//! entries are its commands enqueued and not yet complete, so its free
//! slots are the queue depth less the slots those entries hold
//! ([`DmaQueue::issuer_view`]). Commands complete in
//! `(completion_time, sequence)` order, which with a fixed latency is
//! issue order for each issuer. Commands without a fence or barrier may
//! complete in any order, so the architecture allows this order. A
//! fence or barrier raises a command's completion time to the floor its
//! ordering sets ([`DmaQueue::ordering_floor`]), whatever the latency
//! model gives.
//!
//! A command with a parameter the MFC refuses enters the queue as an
//! invalid entry. The queue reaches it in the same order as a transfer
//! and raises it. Then:
//!
//! - the issuer's queue suspends
//! - the entry keeps its slot and its tag
//! - no later command of that issuer completes
//!
//! A transfer whose address does not translate becomes an invalid entry
//! when the queue reaches it, and raises the same way.
//!
//! [CBEA p:113 s:9.1.1] an invalid command or parameter suspends SPU command queue processing and raises an invalid-command interrupt.
//!
//! [CBEA p:53 s:7.1.1] unless a form says otherwise, data-transfer commands execute in any order.
//
// [CBE-Handbook p:509 s:19] MFC command queues; out-of-order execution; tag-group ordering via fence/barrier.
// [CBE-Handbook p:504 s:18.10.4] 16-entry MFC SPU command queue depth.
// [CBE-Handbook p:522 s:19.3.3.2] 8-entry MFC proxy command queue for PPE-issued commands.

use crate::command::{InvalidMfcCommand, MfcCommandError};
use crate::completion::DmaCompletion;
use crate::request::{DmaRequest, MfcOrdering};
use cellgov_event::UnitId;
use cellgov_mem::lanes::{source, LaneMap, LaneValue, ObjectLanes};
use cellgov_time::GuestTicks;

/// Completion plus optional inline bytes for transfers from
/// unit-private memory.
#[derive(Debug, Clone)]
struct QueueEntry {
    completion: DmaCompletion,
    payload: Option<Vec<u8>>,
}

/// Lane fields of one queued completion:
///
/// 1. the completion time
/// 2. the direction, plus 1
/// 3. the source start
/// 4. the destination start
/// 5. the length
/// 6. the issuer
/// 7. the tag's status bit, 0 without a tag
/// 8. 1 when an inline payload is present
/// 9. the payload bytes
/// 10. the ordering, when the command sets one
/// 11. 1 for a list element with the stall-and-notify flag
/// 12. 1 for a list element that holds no command-queue slot
impl LaneValue for QueueEntry {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        let c = self.completion;
        lanes.lane(1, 0, c.completion_time().raw());
        lanes.lane(2, 0, c.direction() as u64 + 1);
        lanes.lane(3, 0, c.source().start().raw());
        lanes.lane(4, 0, c.destination().start().raw());
        lanes.lane(5, 0, c.length());
        lanes.lane(6, 0, c.issuer().raw());
        let tag = c.request().tag_id().map_or(0, |tag| tag.status_bit());
        lanes.lane(7, 0, u64::from(tag));
        if let Some(payload) = &self.payload {
            lanes.lane(8, 0, 1);
            lanes.bytes(9, &[], payload);
        }
        let ordering = c.request().ordering();
        if ordering != MfcOrdering::None {
            lanes.lane(10, 0, ordering as u64);
        }
        if c.request().stall_notify() {
            lanes.lane(11, 0, 1);
        }
        if !c.request().holds_slot() {
            lanes.lane(12, 0, 1);
        }
    }
}

/// A queued command the MFC refuses when it processes it.
#[derive(Debug, Clone)]
struct InvalidEntry {
    issuer: UnitId,
    command: InvalidMfcCommand,
    /// When the queue reaches the command.
    time: GuestTicks,
    /// The queue reached the command, and the issuer's queue suspended.
    raised: bool,
    /// The refused transfer's [`DmaRequest::holds_slot`].
    holds_slot: bool,
    /// The refused transfer's [`DmaRequest::stall_notify`]; see
    /// [`DmaQueue::stall_notify_tags`].
    stall_notify: bool,
}

impl InvalidEntry {
    /// The status bit of the tag group the command holds outstanding, 0
    /// when its tag is the parameter it fails on.
    fn tag_bit(&self) -> u32 {
        u8::try_from(self.command.params.tag)
            .ok()
            .and_then(cellgov_ps3_abi::hw::spu::MfcTagId::new)
            .map_or(0, |tag| tag.status_bit())
    }
}

/// Lane fields of one invalid command:
///
/// 1. the issuer
/// 2. the tag's status bit, 0 without a valid tag
/// 3. the error code
/// 4. 1 once raised
/// 5. the command word
/// 6. the time the queue reaches it
/// 7. the local-store address
/// 8. the effective address
/// 9. the size
/// 10. the tag channel value
/// 11. 1 for a refused list element with the stall-and-notify flag
/// 12. 1 for a refused list element that holds no command-queue slot
impl LaneValue for InvalidEntry {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        let params = self.command.params;
        lanes.lane(1, 0, self.issuer.raw());
        lanes.lane(2, 0, u64::from(self.tag_bit()));
        lanes.lane(3, 0, u64::from(self.command.error.code()));
        lanes.lane(4, 0, u64::from(self.raised));
        lanes.lane(5, 0, u64::from(self.command.word));
        lanes.lane(6, 0, self.time.raw());
        lanes.lane(7, 0, u64::from(params.lsa));
        lanes.lane(8, 0, params.ea());
        lanes.lane(9, 0, u64::from(params.size));
        lanes.lane(10, 0, u64::from(params.tag));
        if self.stall_notify {
            lanes.lane(11, 0, 1);
        }
        if !self.holds_slot {
            lanes.lane(12, 0, 1);
        }
    }
}

/// The refused command that a queued transfer stands for, with `error`.
///
/// The queue keeps a transfer as ranges, so this function rebuilds the
/// opcode and the two addresses from them. A put with no inline payload
/// names no local-store address, so its `lsa` is 0.
fn refused_transfer(
    c: &DmaCompletion,
    payloaded: bool,
    error: MfcCommandError,
) -> InvalidMfcCommand {
    use crate::request::DmaDirection;
    use cellgov_ps3_abi::hw::spu::{MFC_GET, MFC_PUT};
    let (word, ls, main) = match c.direction() {
        DmaDirection::Put => (MFC_PUT, payloaded.then(|| c.source()), c.destination()),
        DmaDirection::Get => (MFC_GET, Some(c.destination()), c.source()),
    };
    let ea = main.start().raw();
    InvalidMfcCommand {
        word,
        params: crate::command::MfcParameters {
            // A local-store range starts below 2^32.
            lsa: ls.map_or(0, |r| r.start().raw() as u32),
            eah: (ea >> 32) as u32,
            eal: ea as u32,
            size: c.length() as u32,
            tag: c.request().tag_id().map_or(0, |t| u32::from(t.raw())),
        },
        error,
    }
}

/// A command the queue raised: its issuer's queue suspended on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RaisedMfcCommand {
    /// The unit whose queue suspended.
    pub issuer: UnitId,
    /// The command and the check it failed.
    pub command: InvalidMfcCommand,
}

/// What one drain of the queue did.
#[derive(Debug, Default)]
pub struct DueCommands {
    /// Transfers that completed, in queue order, each with its inline
    /// payload.
    pub completions: Vec<(DmaCompletion, Option<Vec<u8>>)>,
    /// Invalid commands the queue reached, in queue order.
    pub raised: Vec<RaisedMfcCommand>,
}

/// Deterministic priority queue of modeled DMA completions.
///
/// Drains in `(completion_time, sequence)` order. [`DmaQueue::enqueue`]
/// and [`DmaQueue::enqueue_invalid`] assign the sequence from one counter.
#[derive(Debug, Clone)]
pub struct DmaQueue {
    entries: LaneMap<(GuestTicks, u64), QueueEntry>,
    invalid: LaneMap<(GuestTicks, u64), InvalidEntry>,
    next_seq: u64,
}

impl Default for DmaQueue {
    fn default() -> Self {
        Self {
            entries: LaneMap::new(source::DMA_QUEUE, |(_, seq)| seq),
            invalid: LaneMap::new(source::MFC_INVALID_COMMAND, |(_, seq)| seq),
            next_seq: 0,
        }
    }
}

impl DmaQueue {
    /// Construct an empty queue.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of queued commands, invalid ones included.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len() + self.invalid.len()
    }

    /// Whether the queue holds no command.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.invalid.is_empty()
    }

    /// Queue `command`, which the MFC refuses, for `issuer`; the queue
    /// reaches it at `time`. Returns the assigned sequence number.
    pub fn enqueue_invalid(
        &mut self,
        time: GuestTicks,
        issuer: UnitId,
        command: InvalidMfcCommand,
    ) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.invalid.insert(
            (time, seq),
            InvalidEntry {
                issuer,
                command,
                time,
                raised: false,
                holds_slot: true,
                stall_notify: false,
            },
        );
        seq
    }

    /// Whether `issuer`'s queue is suspended on a raised command.
    pub fn suspended(&self, issuer: UnitId) -> bool {
        self.invalid
            .values()
            .any(|entry| entry.raised && entry.issuer == issuer)
    }

    /// The earliest time the queue completes or raises a command.
    ///
    /// It skips every command of a suspended issuer: the queue holds it.
    pub fn next_event_time(&self) -> Option<GuestTicks> {
        let transfer = self
            .entries
            .iter()
            .find(|(_, e)| !self.suspended(e.completion.issuer()))
            .map(|((time, _), _)| time);
        let invalid = self
            .invalid
            .iter()
            .find(|(_, e)| !e.raised && !self.suspended(e.issuer))
            .map(|((time, _), _)| time);
        match (transfer, invalid) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Enqueue `completion` with optional inline `payload`, returning
    /// the assigned sequence number.
    ///
    /// When `payload` is `Some`, the commit pipeline uses those bytes
    /// at completion time instead of reading from the source address.
    /// This supports transfers from unit-private memory (e.g. SPU local
    /// store) that is not mapped into the guest address space.
    pub fn enqueue(&mut self, completion: DmaCompletion, payload: Option<Vec<u8>>) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.entries.insert(
            (completion.completion_time(), seq),
            QueueEntry {
                completion,
                payload,
            },
        );
        seq
    }

    /// The earliest completion time that fence and barrier ordering allows `request`.
    ///
    /// The floor is the latest completion time of these queued commands
    /// of the same issuer:
    ///
    /// - a fence or a tag barrier: every command of its tag group
    /// - the barrier command: every command
    /// - behind a queued tag barrier of its group: every command of the
    ///   group that precedes that barrier
    /// - behind a queued barrier command: that barrier command and every
    ///   command that precedes it
    ///
    /// A completed command left the queue at or before the present, so it
    /// sets no floor.
    pub fn ordering_floor(&self, request: &DmaRequest) -> GuestTicks {
        let issuer = request.issuer();
        let tag = request.tag_id();
        let mine = || {
            self.entries
                .iter()
                .filter(move |(_, e)| e.completion.issuer() == issuer)
        };
        let latest = |upto: Option<u64>, same_tag: bool| {
            mine()
                .filter(|((_, seq), e)| {
                    upto.is_none_or(|limit| *seq < limit)
                        && (!same_tag || e.completion.request().tag_id() == tag)
                })
                .map(|((time, _), _)| time)
                .max()
                .unwrap_or(GuestTicks::ZERO)
        };
        let own = match request.ordering() {
            MfcOrdering::None => GuestTicks::ZERO,
            MfcOrdering::Fence | MfcOrdering::TagBarrier => latest(None, true),
            MfcOrdering::QueueBarrier => latest(None, false),
        };
        mine()
            .filter_map(|((_, seq), e)| match e.completion.request().ordering() {
                MfcOrdering::TagBarrier if e.completion.request().tag_id() == tag => {
                    Some(latest(Some(seq), true))
                }
                // Later commands begin when the barrier command itself
                // completes.
                // [CBEA p:72 s:7.9.3] subsequent commands in the queue begin when the barrier command completes.
                MfcOrdering::QueueBarrier => Some(latest(Some(seq + 1), false)),
                _ => None,
            })
            .fold(own, GuestTicks::max)
    }

    /// For one issuer: how many command-queue slots its queued commands
    /// hold, and the status bit of every tag group one of them holds
    /// outstanding.
    ///
    /// An invalid command holds a slot and its tag like a transfer does.
    /// A list's elements hold its tag, and one slot between them.
    pub fn issuer_view(&self, issuer: UnitId) -> (u32, u32) {
        let transfers = self
            .entries
            .values()
            .filter(|e| e.completion.issuer() == issuer)
            .map(|e| {
                let request = e.completion.request();
                let tag = request.tag_id().map_or(0, |tag| tag.status_bit());
                (u32::from(request.holds_slot()), tag)
            });
        let invalid = self
            .invalid
            .values()
            .filter(|e| e.issuer == issuer)
            .map(|e| (u32::from(e.holds_slot), e.tag_bit()));
        transfers
            .chain(invalid)
            .fold((0, 0), |(count, tags), (slot, tag)| {
                (count + slot, tags | tag)
            })
    }

    /// For one issuer: the status bit of every tag group with a queued
    /// list element that carries the stall-and-notify flag.
    ///
    /// A flagged element the queue refused stays in the view: the queue
    /// suspends on it, so its list never stalls.
    pub fn stall_notify_tags(&self, issuer: UnitId) -> u32 {
        let queued = self
            .entries
            .values()
            .map(|e| e.completion.request())
            .filter(|r| r.issuer() == issuer && r.stall_notify())
            .fold(0, |tags, r| {
                tags | r.tag_id().map_or(0, |tag| tag.status_bit())
            });
        let refused = self
            .invalid
            .values()
            .filter(|e| e.issuer == issuer && e.stall_notify)
            .fold(0, |tags, e| tags | e.tag_bit());
        queued | refused
    }

    /// Borrow the earliest pending completion without removing it.
    pub fn peek(&self) -> Option<&DmaCompletion> {
        self.entries.values().next().map(|e| &e.completion)
    }

    /// Every pending completion, in the order the queue drains them,
    /// each with whether an inline payload already carries its bytes.
    ///
    /// A payloaded transfer never reads its source at completion, so a
    /// caller that reasons about what the landing touches needs the
    /// flag as well as the ranges.
    pub fn pending(&self) -> impl Iterator<Item = (&DmaCompletion, bool)> + '_ {
        self.entries
            .values()
            .map(|e| (&e.completion, e.payload.is_some()))
    }

    /// Remove and return the earliest pending completion.
    pub fn pop_next(&mut self) -> Option<(DmaCompletion, Option<Vec<u8>>)> {
        self.entries
            .pop_first()
            .map(|(_, e)| (e.completion, e.payload))
    }

    /// Drain every completion with `completion_time <= now`, in
    /// `(time, sequence)` order. The completions of a suspended issuer
    /// stay queued; see [`Self::process_due`] for the invalid commands.
    pub fn pop_due(&mut self, now: GuestTicks) -> Vec<(DmaCompletion, Option<Vec<u8>>)> {
        self.process_due(now).completions
    }

    /// [`Self::process_due_translating`] with every address translating.
    pub fn process_due(&mut self, now: GuestTicks) -> DueCommands {
        self.process_due_translating(now, |_, _| None)
    }

    /// Process everything due by `now`, in `(time, sequence)` order.
    ///
    /// For each issuer whose queue is not suspended:
    ///
    /// - a transfer completes and leaves the queue
    /// - the queue raises an invalid command, and the issuer's queue
    ///   suspends from there on
    ///
    /// A raised command and every later command of its issuer stay queued.
    ///
    /// The queue calls `translate` on each transfer it reaches, with a
    /// flag that is `true` when an inline payload carries the bytes.
    /// `translate` returns the fault of an address that does not
    /// translate. The queue then raises that transfer as a refused
    /// command and moves none of its bytes.
    ///
    /// [CBEA p:118 s:9.1.6] an invalid effective address suspends the queue; the address is checked during the transfer, so a partial transfer may come first.
    pub fn process_due_translating(
        &mut self,
        now: GuestTicks,
        mut translate: impl FnMut(&DmaCompletion, bool) -> Option<MfcCommandError>,
    ) -> DueCommands {
        let mut due = DueCommands::default();
        loop {
            let transfer = self
                .entries
                .iter()
                .find(|(_, e)| !self.suspended(e.completion.issuer()))
                .map(|(key, _)| key)
                .filter(|(time, _)| *time <= now);
            let invalid = self
                .invalid
                .iter()
                .find(|(_, e)| !e.raised && !self.suspended(e.issuer))
                .map(|(key, _)| key)
                .filter(|(time, _)| *time <= now);
            match (transfer, invalid) {
                (None, None) => return due,
                (Some(t), Some(i)) if t < i => self.complete(t, &mut translate, &mut due),
                (Some(t), None) => self.complete(t, &mut translate, &mut due),
                (_, Some(i)) => self.raise(i, &mut due),
            }
        }
    }

    fn complete(
        &mut self,
        key: (GuestTicks, u64),
        translate: &mut impl FnMut(&DmaCompletion, bool) -> Option<MfcCommandError>,
        due: &mut DueCommands,
    ) {
        let Some(e) = self.entries.remove(key) else {
            return;
        };
        let Some(error) = translate(&e.completion, e.payload.is_some()) else {
            due.completions.push((e.completion, e.payload));
            return;
        };
        let command = refused_transfer(&e.completion, e.payload.is_some(), error);
        let issuer = e.completion.issuer();
        let request = e.completion.request();
        self.invalid.insert(
            key,
            InvalidEntry {
                issuer,
                command,
                time: key.0,
                raised: true,
                holds_slot: request.holds_slot(),
                stall_notify: request.stall_notify(),
            },
        );
        due.raised.push(RaisedMfcCommand { issuer, command });
    }

    fn raise(&mut self, key: (GuestTicks, u64), due: &mut DueCommands) {
        if let Some(mut entry) = self.invalid.get_mut(key) {
            entry.raised = true;
            due.raised.push(RaisedMfcCommand {
                issuer: entry.issuer,
                command: entry.command,
            });
        }
    }

    /// The queue's partial of the sync-state sum, with the sequence
    /// number of each queued command as its object.
    ///
    /// A queue that never held an invalid command has the partial of its
    /// transfers alone: an empty lane map adds nothing.
    #[inline]
    pub fn sync_partial(&self) -> u128 {
        self.entries.partial().wrapping_add(self.invalid.partial())
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub fn sync_partial_from_scratch(&self) -> u128 {
        self.entries
            .partial_from_scratch()
            .wrapping_add(self.invalid.partial_from_scratch())
    }
}

#[cfg(test)]
#[path = "tests/queue_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/queue_lanes_tests.rs"]
mod lanes_tests;

#[cfg(test)]
#[path = "tests/queue_list_tests.rs"]
mod list_tests;

#[cfg(test)]
#[path = "tests/queue_invalid_tests.rs"]
mod invalid_tests;

#[cfg(test)]
#[path = "tests/queue_ordering_tests.rs"]
mod ordering_tests;
