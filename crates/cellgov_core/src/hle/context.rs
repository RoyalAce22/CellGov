//! HLE context trait -- the interface every HLE module operates through.
//!
//! Decouples HLE function implementations from the Runtime struct.
//! Each method corresponds to one capability an HLE handler needs:
//! reading/writing guest memory, returning values, modifying
//! registers, managing the bump allocator, and allocating kernel
//! object IDs.
//!
//! The dispatch layer in `hle.rs` constructs an internal adapter
//! that implements this trait by borrowing from the Runtime.
//! Per-module files (`hle::sys_prx_for_user`, `hle::cell_gcm_sys`, and future modules)
//! accept `&mut dyn HleContext` and never see the Runtime.
//!
//! ## Failure policy
//!
//! This trait exists inside a determinism oracle; silent failures
//! here become divergences the oracle cannot explain. Every method
//! whose underlying operation can fail returns a result type that
//! forces callers to acknowledge the failure:
//!
//! - [`HleContext::write_guest`] returns `Result<(), HleWriteError>`.
//! - [`HleContext::heap_alloc`] returns `Option<u32>` (None on
//!   exhaustion or overflow).
//! - [`HleContext::alloc_id`] returns `Option<u32>` (None when the
//!   monotonic counter overflows).
//!
//! Handlers that cannot recover should `.expect(...)` with a
//! descriptive message; the process aborts at the failing step,
//! which is the behavior the oracle wants.

use std::fmt;

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;

/// Why a call to [`HleContext::write_guest`] did not commit any
/// bytes. Promoting the failure into the type system makes silent
/// drops of commit errors impossible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HleWriteError {
    /// `addr..(addr + bytes.len())` could not form a valid
    /// [`cellgov_mem::ByteRange`] (overflow or empty).
    InvalidRange,
    /// The range was valid, but the underlying
    /// [`cellgov_mem::GuestMemory::apply_commit`] call rejected
    /// the write (unmapped region, length mismatch, etc.).
    CommitFailed(cellgov_mem::MemError),
}

impl fmt::Display for HleWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRange => {
                f.write_str("HLE write: ill-formed byte range (address/length overflow or empty)")
            }
            Self::CommitFailed(err) => write!(f, "HLE write: commit rejected ({err:?})"),
        }
    }
}

impl std::error::Error for HleWriteError {}

/// The interface HLE module implementations operate through.
///
/// Runtime implements this via an internal adapter struct. Tests can mock
/// it. Future module crates depend only on this trait, never on
/// Runtime directly.
pub trait HleContext {
    /// Read-only view of committed guest memory.
    fn guest_memory(&self) -> &[u8];

    /// Guest memory size in bytes.
    fn guest_memory_len(&self) -> usize {
        self.guest_memory().len()
    }

    /// Write bytes to guest memory at the given guest address.
    ///
    /// Returns `Err(HleWriteError)` if the range is ill-formed or
    /// the underlying commit fails. Unlike the prior `let _ = ...`
    /// pattern, the error is surfaced so the handler either
    /// acknowledges it (`.expect(...)`) or forwards it.
    fn write_guest(&mut self, addr: u64, bytes: &[u8]) -> Result<(), HleWriteError>;

    /// Set the syscall return value (lands in r3 on resume).
    fn set_return(&mut self, value: u64);

    /// Push a register write (e.g. r13 for TLS base). Panics if
    /// `reg >= 32` -- PowerPC has exactly 32 GPRs and any out-of-
    /// range index is a programming error in the handler.
    fn set_register(&mut self, reg: u8, value: u64);

    /// Mark the calling unit as Finished (for sys_process_exit).
    fn set_unit_finished(&mut self);

    /// Bump-allocate from the HLE heap.
    ///
    /// Returns `Some(aligned_addr)` on success, `None` if the
    /// arena is exhausted or the request would overflow u32 math.
    /// `align` is rounded up to the next power of two; a caller
    /// passing 0 or 1 effectively requests no alignment.
    fn heap_alloc(&mut self, size: u32, align: u32) -> Option<u32>;

    /// Allocate a monotonic kernel-object ID. Returns `None` if
    /// the counter would wrap past `u32::MAX` -- the prior
    /// `wrapping_add` would have silently issued duplicate IDs
    /// and created use-after-free-shaped kernel-object aliasing.
    fn alloc_id(&mut self) -> Option<u32>;
}

/// Adapter that implements [`HleContext`] by borrowing from a
/// [`Runtime`](crate::runtime::Runtime). Constructed per-dispatch in
/// `dispatch_hle`, dropped immediately after the handler returns.
///
/// ## Mutation witness
///
/// The adapter tracks whether any state-mutating method was called
/// in `mutated`. On drop:
///
/// - Debug builds: `debug_assert!` fires if no mutation happened --
///   the "forgot set_return" footgun that would otherwise leave r3
///   holding a stale value and produce a silent divergence.
/// - Release builds: the NID is added to
///   [`crate::hle::HleState::handlers_without_mutation`], giving
///   production runs the same signal as a counter rather than a
///   panic. Same shape as `unclaimed_nids` -- the operator can
///   walk both maps after a run to distinguish "unimplemented
///   library entry" from "implemented but no-op when it shouldn't
///   be."
///
/// Both paths skip during an active unwind (standard Drop-guard
/// idiom: panicking during unwind aborts the process and swallows
/// the real error).
///
/// There is deliberately no opt-out for handlers that wish to
/// short-circuit the mutation check. If a future test or handler
/// genuinely needs one, add a `#[cfg(test)]`-gated method with a
/// `&'static str` reason logged to stderr -- production handlers
/// must never be able to paper over the Drop guard.
pub(crate) struct RuntimeHleAdapter<'a> {
    pub(crate) memory: &'a mut cellgov_mem::GuestMemory,
    pub(crate) registry: &'a mut crate::registry::UnitRegistry,
    /// Snapshot of `HleState::heap_base`. Read-only from the
    /// adapter's point of view; used to convert the raw `heap_ptr`
    /// into a "bytes handed out" figure for the watermark band
    /// check in `heap_alloc`.
    pub(crate) heap_base: u32,
    pub(crate) heap_ptr: &'a mut u32,
    pub(crate) heap_watermark: &'a mut u32,
    /// Bitmask of heap-watermark bands already reported (see
    /// [`crate::hle::HleState::heap_warning_mask`]). The adapter
    /// reads and writes this so the threshold warning survives
    /// adapter reconstruction across dispatches.
    pub(crate) heap_warning_mask: &'a mut u8,
    pub(crate) next_id: &'a mut u32,
    pub(crate) source: UnitId,
    /// The NID this adapter was constructed to dispatch. Carried so
    /// the Drop impl can attribute a no-mutation miss to a specific
    /// PS3 library entry in the release-mode counter.
    pub(crate) nid: u32,
    /// Set on every mutating call (set_return, set_register,
    /// write_guest, set_unit_finished, heap_alloc, alloc_id). The
    /// Drop guard reads this to detect handlers that constructed
    /// an adapter and then forgot to touch any guest-visible
    /// state. In tests, `mark_unused` also flips it.
    pub(crate) mutated: bool,
    /// Mutable reference to the release-mode no-mutation counter on
    /// [`crate::hle::HleState`]. The Drop impl bumps this when
    /// `mutated` is false and no unwind is in progress.
    pub(crate) handlers_without_mutation: &'a mut std::collections::BTreeMap<u32, usize>,
}

impl Drop for RuntimeHleAdapter<'_> {
    fn drop(&mut self) {
        // Skip during an active unwind. A handler that panics
        // mid-execution never reaches its set_return; firing here
        // would double-panic and abort the process on Windows
        // (STATUS_STACK_BUFFER_OVERRUN), swallowing the real error.
        // Standard Drop-guard idiom.
        if std::thread::panicking() {
            return;
        }
        if self.mutated {
            return;
        }
        // Release-mode: bump the per-NID no-mutation counter so a
        // run-game summary can surface the silent-divergence NID.
        // Debug-mode: same counter bump AND then debug_assert so
        // CI catches regressions loudly. Populating the counter in
        // both modes keeps the observability contract the same;
        // the debug_assert is the test-time amplifier.
        *self.handlers_without_mutation.entry(self.nid).or_insert(0) += 1;
        debug_assert!(
            false,
            "RuntimeHleAdapter dropped without any mutation -- handler for NID {:#010x} \
             (source {:?}) constructed the adapter but never called \
             set_return/set_register/write_guest/set_unit_finished. The guest's r3 still \
             holds its prior value; this is the silent-divergence footgun the Drop guard \
             exists to catch.",
            self.nid, self.source
        );
    }
}

/// Watermark bands the bump allocator reports on crossing. One-shot
/// per band per HleState instance. Ordered low to high so the check
/// loop reports ascending thresholds predictably.
const HEAP_WATERMARK_BANDS: &[(u32, u8, &str)] = &[
    (1 << 20, 0x01, "1 MiB"),
    (10 << 20, 0x02, "10 MiB"),
    (100 << 20, 0x04, "100 MiB"),
];

impl HleContext for RuntimeHleAdapter<'_> {
    fn guest_memory(&self) -> &[u8] {
        self.memory.as_bytes()
    }

    fn write_guest(&mut self, addr: u64, bytes: &[u8]) -> Result<(), HleWriteError> {
        self.mutated = true;
        let range =
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), bytes.len() as u64)
                .ok_or(HleWriteError::InvalidRange)?;
        self.memory
            .apply_commit(range, bytes)
            .map_err(HleWriteError::CommitFailed)
    }

    fn set_return(&mut self, value: u64) {
        self.mutated = true;
        self.registry.set_syscall_return(self.source, value);
    }

    fn set_register(&mut self, reg: u8, value: u64) {
        assert!(
            reg < 32,
            "HleContext::set_register: reg={reg} is out of range (PowerPC has 32 GPRs)"
        );
        self.mutated = true;
        self.registry.push_register_write(self.source, reg, value);
    }

    fn set_unit_finished(&mut self) {
        self.mutated = true;
        self.registry
            .set_status_override(self.source, UnitStatus::Finished);
    }

    fn heap_alloc(&mut self, size: u32, align: u32) -> Option<u32> {
        self.mutated = true;
        // Normalize align to a power of two. u32::next_power_of_two
        // panics in debug / overflows in release for inputs >= 2^31+1,
        // so use the checked variant; fall back to "no alignment"
        // (align=1) on pathological inputs. Prevents a guest-supplied
        // align in `sys_memalign` from crashing the oracle. The
        // fallback 1 is itself a power of two, so the invariant below
        // holds unconditionally.
        let align = align.checked_next_power_of_two().unwrap_or(1);
        debug_assert!(align.is_power_of_two());
        let mask = align - 1;
        let aligned = self.heap_ptr.checked_add(mask)? & !mask;
        let new_ptr = aligned.checked_add(size)?;
        // Compare in u64 to avoid any u32/usize ambiguity on
        // hypothetical hosts where `usize < u64` (bounds-check
        // correctness is independent of the host word size).
        if (new_ptr as u64) <= self.memory.as_bytes().len() as u64 {
            *self.heap_ptr = new_ptr;
            if new_ptr > *self.heap_watermark {
                *self.heap_watermark = new_ptr;
            }
            // Structural invariant: the allocator must never hand
            // out address 0. The dispatch witnesses in
            // `hle::cell_gcm_sys::GcmState` rely on it (see
            // `control_addr != 0` etc.). A zero return would only
            // happen if `heap_base` were deliberately set to 0 --
            // which is a misconfiguration, not a legal oracle state.
            debug_assert_ne!(
                aligned, 0,
                "HLE heap_alloc: allocator returned address 0 (heap_base must be nonzero; \
                 dispatch witnesses depend on this)"
            );
            // Watermark band check. One-shot per band per HleState.
            // `saturating_sub` guards against a caller who reset
            // heap_ptr below heap_base (not a supported pattern,
            // but cheap to defend here so the threshold math never
            // wraps).
            let used = self.heap_watermark.saturating_sub(self.heap_base);
            for &(threshold, bit, label) in HEAP_WATERMARK_BANDS {
                if used >= threshold && (*self.heap_warning_mask & bit) == 0 {
                    *self.heap_warning_mask |= bit;
                    eprintln!(
                        "HLE heap_alloc: watermark crossed {label} above heap_base \
                         ({used} bytes cumulative, bump-on-free allocator); \
                         consider a real allocator -- see NID_SYS_FREE TODO in hle::sys_prx_for_user"
                    );
                }
            }
            Some(aligned)
        } else {
            None
        }
    }

    fn alloc_id(&mut self) -> Option<u32> {
        self.mutated = true;
        // 0 is the exhausted sentinel. After issuing u32::MAX,
        // `next_id` parks at 0 (via `checked_add(1).unwrap_or(0)`)
        // and subsequent calls return None. Reaching the sentinel
        // requires handing out every u32 in [0x8000_0001, u32::MAX]
        // -- roughly 2^31 HLE kernel objects in a single run. No
        // realistic scenario ever comes near that, but "realistic"
        // is not a structural guarantee, and silently recycling IDs
        // would produce kernel-object aliasing (the same u32 key
        // aliasing a freed mutex and a live semaphore, say). The
        // explicit sentinel + None return path forces the oracle
        // to fail loudly instead.
        //
        // The debug_assert below catches misconfigured test
        // fixtures that construct a RuntimeHleAdapter with
        // `next_id: &mut 0` directly, which would otherwise
        // silently never issue any IDs.
        let id = *self.next_id;
        debug_assert_ne!(
            id, 0,
            "alloc_id: next_id hit sentinel 0; did initialization skip 0x8000_0001?"
        );
        if id == 0 {
            return None;
        }
        *self.next_id = id.checked_add(1).unwrap_or(0);
        Some(id)
    }
}
