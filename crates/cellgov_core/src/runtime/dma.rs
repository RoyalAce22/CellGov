//! DMA completion handling. [`Runtime::fire_dma_completions`] runs per
//! commit and wakes issuers whose modeled-latency window arrived.
//! [`Runtime::drain_pending_dma`] runs at scenario termination and
//! lands every outstanding transfer that no suspended queue holds.

use std::borrow::Cow;

use cellgov_dma::{DmaCompletion, MfcCommandError};
use cellgov_exec::UnitStatus;
use cellgov_mem::{ByteRange, GuestMemory};
use cellgov_ps3_abi::hw::ppu::CELL_EA_LIMIT;
use cellgov_time::GuestTicks;
use cellgov_trace::HostWriter;

use super::spaces::AddressSpaceId;
use super::spu_window::{window_target, WindowTarget};
use super::Runtime;

/// The fault a transfer raises when the queue reaches it, or `None`.
///
/// A main-storage range faults when:
///
/// - it runs past the effective-address space (a data-segment fault);
/// - space 0 has no region for it, or a region the access may not use
///   (a data-storage fault).
///
/// CellGov models no segment table. It treats an address past the Cell's
/// real-address bound as one that no segment names, because no region
/// can back it. The architecture does not fix that bound; CellGov does.
/// [CBEA p:120 s:9.1.7] a segment fault raises the MFC data-segment interrupt; a mapping fault or a protection violation raises the MFC data-storage interrupt.
pub(super) fn translation_fault(
    memory: &GuestMemory,
    c: &DmaCompletion,
    payloaded: bool,
) -> Option<MfcCommandError> {
    let check = |range: ByteRange, write: bool| {
        if range.length() == 0 {
            return None;
        }
        let ea = range.start().raw();
        // `ByteRange` holds only ranges that fit the 64-bit space.
        let last = ea + (range.length() - 1);
        if last > CELL_EA_LIMIT {
            return Some(MfcCommandError::DataSegment { ea });
        }
        let translates = if write {
            memory
                .validate_write(range, range.length() as usize)
                .is_ok()
        } else {
            memory.with_reads_unlogged(|mem: &GuestMemory| mem.read_checked(range).is_ok())
        };
        (!translates).then_some(MfcCommandError::DataStorage { ea })
    };
    let request = c.request();
    request
        .main_storage_read(payloaded)
        .and_then(|range| check(range, false))
        .or_else(|| {
            request
                .main_storage_write()
                .and_then(|range| check(range, true))
        })
}

impl Runtime {
    /// Commit one DMA completion's payload to its destination.
    ///
    /// Infallible by construction: the queue hands over only a transfer
    /// whose ranges [`translation_fault`] passed at this instant, and the
    /// runtime applies it before anything else touches memory.
    ///
    /// The write sweeps overlapping reservations. DMA commits
    /// independently of `SharedWriteIntent`. Without the sweep, a
    /// cross-unit MFC_PUT leaves a stale reservation, and a later
    /// `stwcx` / `putllc` reads it as still held.
    /// [PPC-Book2 p:10 s:1.7.3.1] the issuer's own reservation is
    /// preserved; only other processors' reservations are invalidated.
    /// [CBE-Handbook p:479 s:18.6.4] names the Cell half of that split:
    /// the atomic unit loses a lock-line reservation when another
    /// processor element or device modifies the line, and the issuer's
    /// own MFC transfer is a local SPE action rather than that outside
    /// entity.
    fn apply_dma_transfer(&mut self, c: &DmaCompletion, payload: Option<&[u8]>) {
        // A transfer of no bytes touches no address, so its ends need not
        // name a writable region and nothing lands.
        // [CBEA p:116 s:9.1.4 MFC Transfer Size or List Size Channel] Zero is a valid MFC transfer size.
        if c.length() == 0 {
            return;
        }
        let window = window_target(self.lv2_host.thread_groups(), c);
        if c.direction() == cellgov_dma::DmaDirection::Get {
            match window {
                Some(Ok(target)) => self.land_in_window(c, target, &[]),
                _ => self.land_get(c),
            }
            return;
        }
        let Some(bytes) = self.put_source(c, payload) else {
            return;
        };
        if let Some(Ok(target)) = window {
            self.land_in_window(c, target, &bytes);
            return;
        }
        // Both ends resolve in space 0 (see the `spaces` module docs);
        // the fanout below is the only part of a transfer that reaches
        // another space.
        self.host_write(
            HostWriter::DmaCompletion,
            AddressSpaceId::BOOT,
            c.destination(),
            &bytes,
            Some(c.issuer()),
        )
        .expect("the queue translated the DMA destination when it reached the transfer");
        // A landing inside a shared view reaches the sibling views the
        // same way a committed store does. The bytes are the same bytes
        // whichever view names them, so a landing that replicated
        // nothing would leave the siblings holding what the transfer
        // replaced. The issuer is exempt there for the reason it is
        // exempt above: every view of the segment shares one
        // reservation granule, so the split the write already applies
        // in space 0 is the split the aliases carry.
        self.fanout_committed_range(AddressSpaceId::BOOT, c.destination(), Some(c.issuer()));
        // A transfer can land over code a unit is executing: a title
        // loading an overlay by MFC transfer is that shape. Predecoded
        // code at the destination, and at every alias the fanout
        // replicated into, is as stale as after a committed store, so
        // the same invalidation runs over every unit.
        let aliases = self.shared_alias_ranges_in(AddressSpaceId::BOOT, c.destination());
        let (dst, len) = (c.destination().start().raw(), c.destination().length());
        for unit in self.registry.code_caches_mut() {
            unit.invalidate_code(dst, len);
            for alias in &aliases {
                unit.invalidate_code(alias.start().raw(), alias.length());
            }
        }
    }

    /// The bytes a completed put writes, read now: its inline payload, the
    /// issuer's local store, or main storage.
    ///
    /// `None` when the issuer has no local store to read.
    fn put_source<'a>(
        &mut self,
        c: &DmaCompletion,
        payload: Option<&'a [u8]>,
    ) -> Option<Cow<'a, [u8]>> {
        if let Some(data) = payload {
            return Some(Cow::Borrowed(data));
        }
        if !c.request().local_store_source() {
            let bytes = self
                .memory
                .read(c.source())
                .expect("the queue translated the DMA source when it reached the transfer");
            return Some(Cow::Owned(bytes.to_vec()));
        }
        // The source is a local-store offset, below 2^32.
        let (lsa, len) = (c.source().start().raw() as u32, c.length() as u32);
        let read = self
            .registry
            .get(c.issuer())
            .ok_or(cellgov_exec::ProblemStateError::UnknownUnit)
            .and_then(|unit| unit.read_local_store(lsa, len));
        match read {
            Ok(bytes) => Some(Cow::Owned(bytes)),
            Err(err) => {
                self.lv2_host.log_invariant_break(
                    "runtime.dma_put_unread",
                    format_args!(
                        "{:?}: a completed MFC put of {len} bytes from local store 0x{lsa:08x} \
                         read no source ({err:?})",
                        c.issuer(),
                    ),
                );
                None
            }
        }
    }

    /// Land a completed transfer whose effective address is in the SPU
    /// thread window of its issuer's group.
    ///
    /// A put writes `bytes`, which its source held at completion, into
    /// the target's local store; a get reads the target's local store
    /// now. A put into a signal register or the inbound mailbox is that
    /// problem-state write, which wakes a target parked on it.
    ///
    /// [CBEA p:72 s:7.9.4] sndsig writes another SPU's signal-notification register through its effective address.
    fn land_in_window(&mut self, c: &DmaCompletion, target: WindowTarget, bytes: &[u8]) {
        let landed = match (c.direction(), target) {
            (cellgov_dma::DmaDirection::Get, WindowTarget::LocalStore { unit, lsa }) => {
                // The destination is a local-store offset, below 2^32.
                let into = c.destination().start().raw() as u32;
                let len = c.length() as u32;
                self.registry
                    .get(unit)
                    .ok_or(cellgov_exec::ProblemStateError::UnknownUnit)
                    .and_then(|source| source.read_local_store(lsa, len))
                    .and_then(|bytes| {
                        self.registry
                            .get_mut(c.issuer())
                            .ok_or(cellgov_exec::ProblemStateError::UnknownUnit)?
                            .land_local_store(into, &bytes)
                    })
            }
            (cellgov_dma::DmaDirection::Get, _) => Ok(()),
            (cellgov_dma::DmaDirection::Put, target) => {
                // A register put is 4 bytes.
                let word = || {
                    let mut value = [0; 4];
                    for (slot, byte) in value.iter_mut().zip(bytes) {
                        *slot = *byte;
                    }
                    u32::from_be_bytes(value)
                };
                match target {
                    WindowTarget::LocalStore { unit, lsa } => self
                        .registry
                        .get_mut(unit)
                        .ok_or(cellgov_exec::ProblemStateError::UnknownUnit)
                        .and_then(|unit| unit.land_local_store(lsa, bytes)),
                    WindowTarget::Signal { unit, register } => {
                        self.write_unit_signal(unit, register, word())
                    }
                    WindowTarget::InboundMailbox { unit } => self.write_unit_in_mbox(unit, word()),
                }
            }
        };
        if let Err(err) = landed {
            self.lv2_host.log_invariant_break(
                "runtime.dma_window_unlanded",
                format_args!(
                    "{:?}: a completed MFC transfer of {} bytes into the SPU thread window \
                     did not land ({err:?})",
                    c.issuer(),
                    c.length(),
                ),
            );
        }
    }

    /// Land a completed get: read its source now and write the bytes
    /// into the issuer's local store.
    ///
    /// The source is read at completion, so a store another unit
    /// committed while the get was in flight is what lands.
    fn land_get(&mut self, c: &DmaCompletion) {
        let bytes = if c.length() == 0 {
            Vec::new()
        } else {
            self.memory
                .read(c.source())
                .expect("the queue translated the get source when it reached the transfer")
                .to_vec()
        };
        // The destination is a local-store offset, below 2^32.
        let lsa = c.destination().start().raw() as u32;
        let landed = match self.registry.get_mut(c.issuer()) {
            Some(unit) => unit.land_local_store(lsa, &bytes),
            None => Err(cellgov_exec::ProblemStateError::UnknownUnit),
        };
        if let Err(err) = landed {
            self.lv2_host.log_invariant_break(
                "runtime.dma_get_unlanded",
                format_args!(
                    "{:?}: a completed MFC get of {} bytes to local store 0x{lsa:08x} did not \n                     land ({err:?}); the issuer checked the range at issue",
                    c.issuer(),
                    c.length(),
                ),
            );
        }
    }

    /// Apply the DMA completions due now and record each refused command
    /// the queue reaches; returns the fired completions for the trace.
    pub(super) fn fire_dma_completions(&mut self) -> Vec<(DmaCompletion, Option<Vec<u8>>)> {
        let (memory, groups) = (&self.memory, self.lv2_host.thread_groups());
        let processed = self
            .dma_queue
            .process_due_translating(self.time, |c, payloaded| match window_target(groups, c) {
                Some(target) => target.err(),
                None => translation_fault(memory, c, payloaded),
            });
        for raised in processed.raised {
            self.record_mfc_exception(raised);
        }
        let due = processed.completions;
        for (c, payload) in &due {
            self.apply_dma_transfer(c, payload.as_deref());
            // The transfer still commits, so the terminal memory
            // snapshot holds the payload. The `Runnable` override below
            // would replace either of these issuer states:
            // - `Finished`: process-exit residue (see the `runtime`
            //   module docs), or an SPU that stopped before its
            //   transfer completed. An MFC put does not block the
            //   issuer.
            // - `Faulted`: the commit pipeline's `pre_validate` refused
            //   a later DmaEnqueue from the issuer, and that mark keeps
            //   the unit off the scheduler.
            // The transfer leaves the queue either way, so an SPU that
            // restarts sees its tag group complete. Whether the MFC
            // continues while its SPU is stopped is unestablished; this
            // is CellGov's choice.
            // [CBEA p:92 s:8.5.1] a stop request stops the SPU's instruction issue; the page says nothing of the MFC.
            if matches!(
                self.registry.effective_status(c.issuer()),
                Some(UnitStatus::Finished | UnitStatus::Faulted)
            ) {
                continue;
            }
            // A unit stalled on another channel stays parked: only that
            // channel's producer gives it a count.
            if self
                .registry
                .get(c.issuer())
                .and_then(|unit| unit.channel_stall())
                .is_some_and(|stall| !stall.wake.ends_on_dma_completion())
            {
                continue;
            }
            self.registry
                .set_status_override(c.issuer(), UnitStatus::Runnable);
        }
        due
    }

    /// Each local-store range `c` reads or writes when it completes, with
    /// the unit that owns it: the issuer's end, and the target's end of
    /// a transfer through the SPU thread window.
    pub fn dma_local_store_ends(
        &self,
        c: &DmaCompletion,
    ) -> Vec<(cellgov_event::UnitId, ByteRange)> {
        let own = c
            .request()
            .local_store_range()
            .map(|range| (c.issuer(), range));
        let window = match window_target(self.lv2_host.thread_groups(), c) {
            Some(Ok(WindowTarget::LocalStore { unit, lsa })) => {
                ByteRange::new(cellgov_mem::GuestAddr::new(u64::from(lsa)), c.length())
                    .map(|range| (unit, range))
            }
            _ => None,
        };
        own.into_iter().chain(window).collect()
    }

    /// For `unit`: how many of its MFC commands are queued, and one bit
    /// for each tag group with one of its transfers in the queue.
    ///
    /// [CBEA p:128 s:9.3.6] a tag group reads complete when it has no outstanding operations.
    pub(super) fn unit_dma_view(&self, unit: cellgov_event::UnitId) -> (u32, u32) {
        self.dma_queue.issuer_view(unit)
    }

    /// One bit for each tag group with a `unit` transfer in the queue.
    #[cfg(test)]
    pub(super) fn outstanding_dma_tags(&self, unit: cellgov_event::UnitId) -> u32 {
        self.unit_dma_view(unit).1
    }

    /// Land the queued transfers, whatever their scheduled time, for the
    /// final memory snapshot at scenario termination.
    ///
    /// The drain reaches a refused command in queue order. That command
    /// suspends its issuer's queue, and the issuer's later transfers never
    /// land.
    pub fn drain_pending_dma(&mut self) {
        let (memory, groups) = (&self.memory, self.lv2_host.thread_groups());
        let processed =
            self.dma_queue
                .process_due_translating(GuestTicks::new(u64::MAX), |c, payloaded| {
                    match window_target(groups, c) {
                        Some(target) => target.err(),
                        None => translation_fault(memory, c, payloaded),
                    }
                });
        for raised in processed.raised {
            self.record_mfc_exception(raised);
        }
        let due = processed.completions;
        for (c, payload) in &due {
            self.apply_dma_transfer(c, payload.as_deref());
        }
    }
}

#[cfg(test)]
#[path = "tests/dma_space_tests.rs"]
mod dma_space_tests;

#[cfg(test)]
#[path = "tests/dma_shared_view_tests.rs"]
mod dma_shared_view_tests;

#[cfg(test)]
#[path = "tests/dma_invalidation_tests.rs"]
mod dma_invalidation_tests;

#[cfg(test)]
#[path = "tests/dma_translation_tests.rs"]
mod dma_translation_tests;

#[cfg(test)]
#[path = "tests/dma_ordering_tests.rs"]
mod dma_ordering_tests;
