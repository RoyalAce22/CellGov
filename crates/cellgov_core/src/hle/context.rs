//! HLE handler interface: per-module handlers see only `&mut dyn HleContext`,
//! never the Runtime.

use std::fmt;

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_lv2::CallbackReturnStage;

/// Failure modes for [`HleContext::write_guest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HleWriteError {
    /// Address plus length overflowed or the range was empty.
    InvalidRange,
    /// Underlying memory commit was rejected.
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

/// Failure modes for [`HleContext::read_guest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HleReadError {
    /// Address plus length overflowed or the range was empty.
    InvalidRange,
    /// Underlying memory read was rejected.
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

/// Park-intent record produced by a handler calling
/// [`HleContext::park_for_callback`].
///
/// Cross-module contract: the handler records intent and returns
/// without spawning. The dispatch path then drains this request,
/// spawns the worker via
/// [`cellgov_lv2::Lv2Host::call_guest_callback_sync`] +
/// [`crate::Runtime::apply_callback_spawn`], and parks the caller on
/// the worker's trampoline return; the parent's r3 is set by the
/// resume arm, never by the handler.
///
/// At most one outstanding request per dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HleParkRequest {
    /// Guest OPD (function descriptor) address of the callback to invoke.
    pub opd_addr: u32,
    /// Worker `r3..=r10` at entry.
    pub args: [u64; 8],
    /// Resume arm that consumes the worker's return value.
    pub stage: CallbackReturnStage,
}

/// Per-dispatch handler interface; the only Runtime surface HLE handlers see.
pub trait HleContext {
    /// Flat base-0 view; ignores region boundaries.
    ///
    /// Safe only for fields the handler itself wrote via
    /// [`Self::write_guest`]. Guest-supplied pointers must go through
    /// [`Self::read_guest`] so unmapped reads fail loud.
    fn guest_memory(&self) -> &[u8];

    /// Length of the flat guest memory view in bytes.
    fn guest_memory_len(&self) -> usize {
        self.guest_memory().len()
    }

    /// Bounds- and region-checked read.
    fn read_guest(&self, addr: u64, len: usize) -> Result<&[u8], HleReadError>;

    /// Bounds- and region-checked write through the commit pipeline.
    fn write_guest(&mut self, addr: u64, bytes: &[u8]) -> Result<(), HleWriteError>;

    /// Set r3 on resume.
    fn set_return(&mut self, value: u64);

    /// # Panics
    ///
    /// Panics if `reg >= 32`.
    fn set_register(&mut self, reg: u8, value: u64);

    /// Mark the calling unit as Finished on resume.
    fn set_unit_finished(&mut self);

    /// Bump-allocate. `align` is rounded up to the next power of two;
    /// 0 or 1 means none. `None` on exhaustion or u32 overflow.
    fn heap_alloc(&mut self, size: u32, align: u32) -> Option<u32>;

    /// Monotonic kernel-object ID. `None` past `u32::MAX`; IDs are
    /// never recycled (aliasing would break kernel-object identity).
    fn alloc_id(&mut self) -> Option<u32>;

    /// Record a park intent; the dispatch path drains it and spawns
    /// the worker after the handler returns. See [`HleParkRequest`]
    /// for the full contract.
    ///
    /// At most one park per dispatch; a second call in the same
    /// handler displaces the first and trips a debug assert.
    fn park_for_callback(&mut self, request: HleParkRequest);
}

/// [`HleContext`] adapter borrowing Runtime state. Constructed per
/// dispatch in `dispatch_hle` and dropped after the handler returns.
///
/// Mutation witness: a clean drop with no mutating method called bumps
/// [`crate::hle::HleState::handlers_without_mutation`] and trips a
/// debug assert.
pub(crate) struct RuntimeHleAdapter<'a> {
    /// Guest memory view; the source of `read_guest`/`write_guest`.
    pub(crate) memory: &'a mut cellgov_mem::GuestMemory,
    /// Unit registry receiving register writes and status overrides.
    pub(crate) registry: &'a mut crate::registry::UnitRegistry,
    /// Lowest legal heap address; watermark warnings are reported above this.
    pub(crate) heap_base: u32,
    /// Bump-allocator cursor.
    pub(crate) heap_ptr: &'a mut u32,
    /// Highest cursor value ever reached.
    pub(crate) heap_watermark: &'a mut u32,
    /// Bitmask of watermark bands already warned on.
    pub(crate) heap_warning_mask: &'a mut u8,
    /// Next monotonic kernel-object ID handed out by `alloc_id`.
    pub(crate) next_id: &'a mut u32,
    /// Unit whose dispatch produced this adapter.
    pub(crate) source: UnitId,
    /// NID being dispatched, used in mutation-witness diagnostics.
    pub(crate) nid: u32,
    /// Set by any mutating method; drop checks this for the witness assert.
    pub(crate) mutated: bool,
    /// Per-NID counter of dispatches that returned without mutating state.
    pub(crate) handlers_without_mutation: &'a mut std::collections::BTreeMap<u32, usize>,
    /// Park intent recorded by `park_for_callback`, drained by dispatch.
    pub(crate) pending_callback_spawn: &'a mut Option<HleParkRequest>,
}

impl Drop for RuntimeHleAdapter<'_> {
    fn drop(&mut self) {
        // Skip during unwind to avoid double-panic abort.
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
             holds its prior value.",
            self.nid, self.source
        );
    }
}

/// One-shot watermark bands, ordered low to high.
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
        // Fall back to 1 so a guest-supplied 0 cannot crash the oracle.
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
            // Cross-module: dispatch witnesses in `hle::cell_gcm_sys::GcmState`
            // treat address 0 as "unset", so heap_base must be nonzero.
            debug_assert_ne!(
                aligned, 0,
                "HLE heap_alloc: allocator returned address 0 (heap_base must be nonzero)"
            );
            let used = self.heap_watermark.saturating_sub(self.heap_base);
            for &(threshold, bit, label) in HEAP_WATERMARK_BANDS {
                if used >= threshold && (*self.heap_warning_mask & bit) == 0 {
                    *self.heap_warning_mask |= bit;
                    #[allow(
                        clippy::print_stderr,
                        reason = "one-shot watermark warning, gated by heap_warning_mask so each band fires at most once per host instance"
                    )]
                    {
                        eprintln!(
                            "HLE heap_alloc: watermark crossed {label} above heap_base \
                             ({used} bytes cumulative, bump-on-free allocator); \
                             consider a real allocator -- see _sys_free TODO in hle::sys_prx_for_user"
                        );
                    }
                }
            }
            Some(aligned)
        } else {
            None
        }
    }

    fn alloc_id(&mut self) -> Option<u32> {
        self.mutated = true;
        // 0 is the exhausted sentinel; reaching it before exhaustion
        // means a fixture forgot to seed next_id.
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

    fn park_for_callback(&mut self, request: HleParkRequest) {
        self.mutated = true;
        let prior = self.pending_callback_spawn.replace(request);
        debug_assert!(
            prior.is_none(),
            "park_for_callback called twice in one dispatch (NID {:#010x}, source {:?}); \
             prior request {:?} was displaced",
            self.nid,
            self.source,
            prior,
        );
    }
}
