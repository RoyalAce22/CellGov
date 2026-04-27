//! HLE context trait: the interface every HLE module operates through.
//!
//! Per-module files accept `&mut dyn HleContext` and never see the
//! Runtime. The dispatch layer in `hle.rs` constructs an internal
//! adapter that implements this trait by borrowing from the Runtime.
//!
//! ## Failure policy
//!
//! Fallible operations return a result type so silent drops become
//! impossible: [`HleContext::write_guest`] is `Result`,
//! [`HleContext::heap_alloc`] and [`HleContext::alloc_id`] are
//! `Option`. Handlers that cannot recover `.expect(...)`.

use std::fmt;

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;

/// Why a call to [`HleContext::write_guest`] did not commit any
/// bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HleWriteError {
    /// `addr..(addr + bytes.len())` could not form a valid
    /// [`cellgov_mem::ByteRange`] (overflow or empty).
    InvalidRange,
    /// The underlying [`cellgov_mem::GuestMemory::apply_commit`]
    /// rejected the write.
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

/// Why a call to [`HleContext::read_guest`] did not return bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HleReadError {
    /// `addr..(addr + len)` could not form a valid `ByteRange`
    /// (overflow or empty).
    InvalidRange,
    /// The underlying [`cellgov_mem::GuestMemory::read_checked`]
    /// rejected the read (unmapped, out-of-range, or strict-reserved).
    ReadFailed(cellgov_mem::MemError),
}

impl fmt::Display for HleReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRange => {
                f.write_str("HLE read: ill-formed byte range (address/length overflow or empty)")
            }
            Self::ReadFailed(err) => write!(f, "HLE read: read rejected ({err:?})"),
        }
    }
}

impl std::error::Error for HleReadError {}

/// The interface HLE module implementations operate through.
pub trait HleContext {
    /// Read-only view of the base-0 region's committed bytes.
    ///
    /// This is the legacy flat accessor: it ignores region boundaries
    /// and treats addresses past the slice end as out-of-range. Only
    /// safe for fields the handler has previously written through
    /// [`Self::write_guest`] (the post-zero-init witness pattern).
    /// For guest-supplied pointers, use [`Self::read_guest`] so an
    /// unmapped or out-of-base-region read fails loud.
    fn guest_memory(&self) -> &[u8];

    /// Guest memory size in bytes (base-0 region).
    fn guest_memory_len(&self) -> usize {
        self.guest_memory().len()
    }

    /// Read `len` bytes from guest memory at `addr`, honoring region
    /// boundaries and access modes. Symmetric counterpart to
    /// [`Self::write_guest`]: bad pointers surface as `Err` rather
    /// than silently substituting zeros.
    fn read_guest(&self, addr: u64, len: usize) -> Result<&[u8], HleReadError>;

    /// Write bytes to guest memory at the given guest address.
    fn write_guest(&mut self, addr: u64, bytes: &[u8]) -> Result<(), HleWriteError>;

    /// Set the syscall return value (lands in r3 on resume).
    fn set_return(&mut self, value: u64);

    /// Push a register write (e.g. r13 for TLS base).
    ///
    /// # Panics
    ///
    /// Panics if `reg >= 32` (PowerPC has 32 GPRs).
    fn set_register(&mut self, reg: u8, value: u64);

    /// Mark the calling unit as Finished (for sys_process_exit).
    fn set_unit_finished(&mut self);

    /// Bump-allocate from the HLE heap.
    ///
    /// `align` is rounded up to the next power of two; 0 or 1 means
    /// no alignment. Returns `None` on exhaustion or u32 overflow.
    fn heap_alloc(&mut self, size: u32, align: u32) -> Option<u32>;

    /// Allocate a monotonic kernel-object ID.
    ///
    /// Returns `None` past `u32::MAX` rather than recycling IDs,
    /// which would produce kernel-object aliasing.
    fn alloc_id(&mut self) -> Option<u32>;
}

/// Adapter implementing [`HleContext`] by borrowing from a
/// [`Runtime`](crate::runtime::Runtime). Constructed per-dispatch in
/// `dispatch_hle` and dropped immediately after the handler returns.
///
/// ## Mutation witness
///
/// On drop, if no mutating method was called and no unwind is
/// active, the NID is added to
/// [`crate::hle::HleState::handlers_without_mutation`]; debug builds
/// additionally `debug_assert!`. The counter is the production
/// signal; the assert is the test-time amplifier.
pub(crate) struct RuntimeHleAdapter<'a> {
    pub(crate) memory: &'a mut cellgov_mem::GuestMemory,
    pub(crate) registry: &'a mut crate::registry::UnitRegistry,
    /// Snapshot of `HleState::heap_base`. `heap_alloc`'s watermark
    /// band check subtracts this from `heap_ptr`.
    pub(crate) heap_base: u32,
    pub(crate) heap_ptr: &'a mut u32,
    pub(crate) heap_watermark: &'a mut u32,
    pub(crate) heap_warning_mask: &'a mut u8,
    pub(crate) next_id: &'a mut u32,
    pub(crate) source: UnitId,
    /// The NID being dispatched. The Drop impl uses it to
    /// attribute a no-mutation miss to a specific library entry.
    pub(crate) nid: u32,
    /// Set on every mutating method. The Drop guard reads this.
    pub(crate) mutated: bool,
    pub(crate) handlers_without_mutation: &'a mut std::collections::BTreeMap<u32, usize>,
}

impl Drop for RuntimeHleAdapter<'_> {
    fn drop(&mut self) {
        // Standard Drop-guard idiom: skip during an active unwind
        // so a double-panic does not abort the process.
        if std::thread::panicking() {
            return;
        }
        if self.mutated {
            return;
        }
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
/// per band per HleState; ordered low to high.
const HEAP_WATERMARK_BANDS: &[(u32, u8, &str)] = &[
    (1 << 20, 0x01, "1 MiB"),
    (10 << 20, 0x02, "10 MiB"),
    (100 << 20, 0x04, "100 MiB"),
];

impl HleContext for RuntimeHleAdapter<'_> {
    fn guest_memory(&self) -> &[u8] {
        self.memory.as_bytes()
    }

    fn read_guest(&self, addr: u64, len: usize) -> Result<&[u8], HleReadError> {
        let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), len as u64)
            .ok_or(HleReadError::InvalidRange)?;
        self.memory
            .read_checked(range)
            .map_err(HleReadError::ReadFailed)
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
        // Normalize align to a power of two; fall back to 1 on
        // pathological guest-supplied values so `sys_memalign`
        // cannot crash the oracle.
        let align = align.checked_next_power_of_two().unwrap_or(1);
        debug_assert!(align.is_power_of_two());
        let mask = align - 1;
        let aligned = self.heap_ptr.checked_add(mask)? & !mask;
        let new_ptr = aligned.checked_add(size)?;
        if (new_ptr as u64) <= self.memory.as_bytes().len() as u64 {
            *self.heap_ptr = new_ptr;
            if new_ptr > *self.heap_watermark {
                *self.heap_watermark = new_ptr;
            }
            // Dispatch witnesses in `hle::cell_gcm_sys::GcmState`
            // rely on the allocator never handing out address 0.
            debug_assert_ne!(
                aligned, 0,
                "HLE heap_alloc: allocator returned address 0 (heap_base must be nonzero; \
                 dispatch witnesses depend on this)"
            );
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
        // 0 is the exhausted sentinel: after issuing u32::MAX,
        // `next_id` parks at 0 and subsequent calls return None.
        // Test fixtures that pass `next_id: &mut 0` directly trip
        // the debug_assert rather than silently issuing no IDs.
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
