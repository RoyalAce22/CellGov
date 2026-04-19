//! PPU thread table.
//!
//! Tracks guest-visible PS3 PPU threads: the primary thread (seeded
//! at host construction) and any additional threads created via
//! `sys_ppu_thread_create`. Each thread has a stable `PpuThreadId`,
//! a `PpuThreadState` (Runnable / Blocked / Finished / Detached),
//! and -- while blocked -- a `GuestBlockReason` that names what it
//! is waiting for. Today the only block reason modeled is
//! `WaitingOnJoin`; richer sync primitives extend the enum as
//! they are added.
//!
//! The table itself is pure data. It is owned by the LV2 host;
//! the runtime never touches it directly. The scheduler only
//! observes `UnitStatus::Blocked` -- the guest reason lives
//! here.

use cellgov_event::UnitId;
use std::collections::BTreeMap;

/// Immutable TLS template captured from the game's PT_TLS program
/// header at ELF load time.
///
/// Every PPU thread gets its own per-thread TLS block that starts
/// life as a byte-wise copy of `initial_bytes`, followed by
/// `mem_size - initial_bytes.len()` zero bytes (the PT_TLS BSS
/// tail). The primary thread's TLS is placed at `vaddr` in guest
/// memory by the loader (via the existing `pre_init_tls` path);
/// child threads created by `sys_ppu_thread_create` allocate their
/// TLS blocks elsewhere and initialize them from this template.
///
/// The template is populated once at boot and never mutated. It is
/// read-only state -- child threads mutating their per-thread TLS
/// blocks have no effect here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TlsTemplate {
    initial_bytes: Vec<u8>,
    mem_size: u64,
    align: u64,
    vaddr: u64,
}

impl TlsTemplate {
    /// The empty template: zero-sized, no initial bytes. Used by
    /// `Lv2Host::new` before an ELF is loaded.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Construct a template from captured ELF PT_TLS data.
    ///
    /// `initial_bytes` is the `.tdata` payload (filesz bytes from
    /// the ELF); `mem_size` is the total per-thread size including
    /// the `.tbss` zero-init tail (memsz); `align` is the segment
    /// alignment; `vaddr` is where the primary thread's TLS landed.
    pub fn new(initial_bytes: Vec<u8>, mem_size: u64, align: u64, vaddr: u64) -> Self {
        debug_assert!(initial_bytes.len() as u64 <= mem_size);
        Self {
            initial_bytes,
            mem_size,
            align,
            vaddr,
        }
    }

    /// Initial (initialized) bytes to stamp into each new thread's
    /// TLS block. Length is `<= mem_size`.
    pub fn initial_bytes(&self) -> &[u8] {
        &self.initial_bytes
    }

    /// Total per-thread TLS block size in bytes. The tail beyond
    /// `initial_bytes.len()` is zero-filled.
    pub fn mem_size(&self) -> u64 {
        self.mem_size
    }

    /// Alignment required for each per-thread TLS block.
    pub fn align(&self) -> u64 {
        self.align
    }

    /// Guest virtual address where the primary thread's TLS was
    /// placed by the loader.
    pub fn vaddr(&self) -> u64 {
        self.vaddr
    }

    /// Whether this template has zero size (no PT_TLS in the ELF,
    /// or `Lv2Host::new` default). Child threads spawned against
    /// an empty template get a zero-length TLS block, which is
    /// correct for games with no PT_TLS segment.
    pub fn is_empty(&self) -> bool {
        self.mem_size == 0 && self.initial_bytes.is_empty()
    }

    /// FNV-1a contribution used by `Lv2Host::state_hash` so that
    /// template mutations (which should never happen after load)
    /// are detected.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&self.mem_size.to_le_bytes());
        hasher.write(&self.align.to_le_bytes());
        hasher.write(&self.vaddr.to_le_bytes());
        hasher.write(&self.initial_bytes);
        hasher.finish()
    }

    /// Instantiate a fresh per-thread TLS block.
    ///
    /// Returns a `Vec<u8>` of length `mem_size` whose first
    /// `initial_bytes.len()` bytes are a copy of the template and
    /// whose remaining bytes are zero (matching the ELF
    /// `.tdata` + `.tbss` layout). Each call produces an
    /// independent buffer; mutations to one thread's TLS block
    /// never affect another's.
    ///
    /// Called by `sys_ppu_thread_create` after
    /// `Lv2Host::allocate_child_stack` has reserved the child's
    /// stack range. The caller copies the returned bytes into
    /// guest memory at the address chosen for the child's TLS.
    pub fn instantiate(&self) -> Vec<u8> {
        let mem = self.mem_size as usize;
        let init = self.initial_bytes.len().min(mem);
        let mut block = vec![0u8; mem];
        if init > 0 {
            block[..init].copy_from_slice(&self.initial_bytes[..init]);
        }
        block
    }
}

/// A reserved stack block for a child PPU thread.
///
/// `base` is the lowest address of the block (stack grows downward
/// from `base + size`). The ABI-required top-of-stack value for
/// the thread's `r1` register is `base + size - 0x10` (16 bytes
/// reserved at the top for the initial back-chain word and the
/// minimum register save area).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadStack {
    /// Lowest address of the reserved block (inclusive).
    pub base: u64,
    /// Block size in bytes.
    pub size: u64,
}

impl ThreadStack {
    /// Top-of-stack address to load into `r1` when starting the
    /// child thread. Leaves 16 bytes reserved above `r1` for the
    /// ABI-required back-chain word and minimum save area.
    pub fn initial_sp(&self) -> u64 {
        self.base + self.size - 0x10
    }

    /// Upper bound of the reserved block (exclusive).
    pub fn end(&self) -> u64 {
        self.base + self.size
    }
}

/// Deterministic allocator for child-thread stack blocks.
///
/// Carves blocks from the `0xD0010000+` region (immediately above
/// the primary thread's 64 KB stack at `0xD0000000-0xD000FFFF`).
/// The primary thread is never allocated here -- it is seeded
/// directly at `PS3_PRIMARY_STACK_BASE` by the boot path. The
/// allocator is a pure bump allocator: two instances with the
/// same construction parameters produce byte-identical allocation
/// sequences.
///
/// The allocator guarantees 16-byte alignment by default (the
/// PowerPC ABI minimum for stack pointers). Callers may request
/// stronger alignment; the allocator rounds `base` up accordingly
/// before committing.
#[derive(Debug, Clone)]
pub struct ThreadStackAllocator {
    next: u64,
}

impl ThreadStackAllocator {
    /// Lowest address the allocator will hand out. Sits
    /// immediately above the primary thread's 64 KB stack at
    /// `0xD0000000-0xD000FFFF`.
    pub const CHILD_STACK_BASE: u64 = 0xD001_0000;

    /// Construct a fresh allocator. The first `allocate` returns
    /// a block starting at or just above `CHILD_STACK_BASE`.
    pub fn new() -> Self {
        Self {
            next: Self::CHILD_STACK_BASE,
        }
    }

    /// Allocate a stack block of `size` bytes. The returned block
    /// is aligned to `max(align, 16)`. Returns `None` on overflow.
    pub fn allocate(&mut self, size: u64, align: u64) -> Option<ThreadStack> {
        let align = align.max(0x10);
        let mask = align - 1;
        let base = self.next.checked_add(mask)? & !mask;
        let end = base.checked_add(size)?;
        self.next = end;
        Some(ThreadStack { base, size })
    }

    /// Peek the address where the next allocation of a given
    /// alignment would start, without advancing the allocator.
    pub fn peek_next(&self, align: u64) -> Option<u64> {
        let align = align.max(0x10);
        let mask = align - 1;
        self.next.checked_add(mask).map(|n| n & !mask)
    }
}

impl Default for ThreadStackAllocator {
    fn default() -> Self {
        Self::new()
    }
}

/// Guest-facing PPU thread id.
///
/// The primary thread is always `PpuThreadId::PRIMARY`
/// (`0x0100_0000`); child threads created via
/// `sys_ppu_thread_create` allocate from `0x0100_0001` upward.
/// Values below `0x0100_0000` are reserved and never handed out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PpuThreadId(u64);

impl PpuThreadId {
    /// The primary thread's id. Reserved -- the allocator never
    /// returns this value.
    pub const PRIMARY: PpuThreadId = PpuThreadId(0x0100_0000);

    /// Construct a thread id from a raw u64. Callers are
    /// responsible for keeping the value inside the PPU-thread
    /// range; this is a thin wrapper, not a validator.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Raw u64 value of this thread id.
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Monotonic allocator for child-thread ids.
///
/// Starts at `0x0100_0001` and hands out strictly increasing ids
/// on each call. Once u64 space is exhausted the allocator
/// returns `None` and stays exhausted; future calls also return
/// `None`. The primary id (`0x0100_0000`) is never returned.
#[derive(Debug, Clone)]
pub struct PpuThreadIdAllocator {
    next: u64,
}

impl PpuThreadIdAllocator {
    /// Construct a fresh allocator. The first call to `allocate`
    /// returns `PpuThreadId(0x0100_0001)`.
    pub fn new() -> Self {
        Self {
            next: PpuThreadId::PRIMARY.raw() + 1,
        }
    }

    /// Allocate the next thread id. Returns `None` once u64 space
    /// is exhausted.
    pub fn allocate(&mut self) -> Option<PpuThreadId> {
        let next_next = self.next.checked_add(1)?;
        let id = PpuThreadId(self.next);
        self.next = next_next;
        Some(id)
    }

    /// Peek the id that the next `allocate` call will return (if
    /// any). Does not advance the allocator.
    pub fn peek(&self) -> Option<PpuThreadId> {
        if self.next == u64::MAX {
            None
        } else {
            Some(PpuThreadId(self.next))
        }
    }
}

impl Default for PpuThreadIdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl PpuThreadIdAllocator {
    /// Construct an allocator whose next-id is `next`. Test-only;
    /// used to exercise exhaustion behavior without allocating
    /// billions of times.
    pub(crate) fn with_next(next: u64) -> Self {
        Self { next }
    }
}

/// Why a PPU thread is currently blocked.
///
/// The thread table owns this guest-semantic reason. Today it
/// models `WaitingOnJoin`; richer sync primitives (mutex /
/// cond / sem / event queue / event flag) extend this enum as
/// they land.
///
/// The scheduler never branches on this -- it sees the opaque
/// `UnitStatus::Blocked` and skips the unit. The LV2 host is
/// responsible for transitioning the unit back to runnable when
/// the underlying condition resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestBlockReason {
    /// Waiting for `target` to call `sys_ppu_thread_exit`.
    WaitingOnJoin {
        /// Thread whose exit this waiter is parked on.
        target: PpuThreadId,
    },
    /// Waiting for the lightweight mutex `id` to become available
    /// (its `sys_lwmutex_unlock`).
    WaitingOnLwMutex {
        /// Guest id of the lightweight mutex being awaited.
        id: u32,
    },
    /// Waiting for the heavy mutex `id` to become available (its
    /// `sys_mutex_unlock`).
    WaitingOnMutex {
        /// Guest id of the heavy mutex being awaited.
        id: u32,
    },
    /// Waiting for a `sys_semaphore_post` on semaphore `id` that
    /// hands ownership of the slot to this waiter.
    WaitingOnSemaphore {
        /// Guest id of the semaphore being awaited.
        id: u32,
    },
    /// Waiting for a `sys_event_queue_send` to deliver a payload on
    /// event queue `id`.
    WaitingOnEventQueue {
        /// Guest id of the event queue being awaited.
        id: u32,
    },
    /// Waiting for a `sys_event_flag_set` that satisfies `mask` per
    /// `mode` on event flag `id`.
    WaitingOnEventFlag {
        /// Guest id of the event flag being awaited.
        id: u32,
        /// Bit mask the waiter's match predicate uses.
        mask: u64,
        /// Match / clear-on-wake policy for this waiter.
        mode: EventFlagWaitMode,
    },
    /// Waiting for a `sys_cond_signal` / `_signal_all` on cond
    /// `cond_id`. On wake, the runtime re-acquires the associated
    /// mutex `mutex_id` before returning control to the caller.
    WaitingOnCond {
        /// Guest id of the cond being awaited.
        cond_id: u32,
        /// Guest id of the mutex released at cond_wait entry; the
        /// wake path re-acquires it (or parks on it if held).
        mutex_id: u32,
    },
}

/// Event-flag wait policy. Encodes the two orthogonal bits the PS3
/// ABI exposes at `sys_event_flag_wait` time: how the mask matches
/// the current bits (AND = all set, OR = any set), and whether the
/// matched bits are cleared on wake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventFlagWaitMode {
    /// All bits in `mask` must be set; do not clear on wake.
    AndNoClear,
    /// All bits in `mask` must be set; clear the matched bits on
    /// wake.
    AndClear,
    /// Any bit in `mask` must be set; do not clear on wake.
    OrNoClear,
    /// Any bit in `mask` must be set; clear the matched bits on
    /// wake.
    OrClear,
}

/// Lifecycle state of a PPU thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PpuThreadState {
    /// Ready to run.
    Runnable,
    /// Parked on a guest-LV2 condition.
    Blocked(GuestBlockReason),
    /// Called `sys_ppu_thread_exit`; exit value is available.
    Finished,
    /// Explicitly detached; resources released on exit without a
    /// join.
    Detached,
}

/// Attributes captured from `sys_ppu_thread_create`.
#[derive(Debug, Clone)]
pub struct PpuThreadAttrs {
    /// Guest entry-point OPD address (function descriptor pointer,
    /// not the code address itself).
    pub entry: u64,
    /// Argument value passed to the entry function in `r3`.
    pub arg: u64,
    /// Bottom of the child thread's stack region. Stack grows
    /// downward from `stack_base + stack_size`.
    pub stack_base: u32,
    /// Size in bytes of the child thread's stack region.
    pub stack_size: u32,
    /// Scheduling priority. Captured from the guest but not
    /// consulted by the current round-robin scheduler.
    pub priority: u32,
    /// Base address of the child thread's per-thread TLS block.
    pub tls_base: u32,
}

/// A single PPU thread tracked by the host.
#[derive(Debug, Clone)]
pub struct PpuThread {
    /// Guest-facing thread id.
    pub id: PpuThreadId,
    /// Runtime execution-unit id. For the primary thread this is
    /// the unit registered at startup; for child threads it is
    /// minted by `sys_ppu_thread_create`.
    pub unit_id: UnitId,
    /// Current lifecycle state.
    pub state: PpuThreadState,
    /// Creation attributes. Immutable after create.
    pub attrs: PpuThreadAttrs,
    /// Value the thread returned via `sys_ppu_thread_exit`, if any.
    pub exit_value: Option<u64>,
    /// Threads waiting on this one's completion. Appended by
    /// `sys_ppu_thread_join`, drained when `mark_finished` fires.
    pub join_waiters: Vec<PpuThreadId>,
}

/// Table of PPU threads owned by the LV2 host.
///
/// The primary thread is inserted exactly once via
/// `insert_primary` at host construction; additional threads are
/// created via `create` in response to `sys_ppu_thread_create`.
/// Lookup by `PpuThreadId` (guest-facing) or `UnitId` (runtime).
#[derive(Debug, Clone, Default)]
pub struct PpuThreadTable {
    allocator: PpuThreadIdAllocator,
    threads: BTreeMap<PpuThreadId, PpuThread>,
    unit_to_thread: BTreeMap<UnitId, PpuThreadId>,
}

impl PpuThreadTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert the primary thread. Must be called exactly once at
    /// host construction, before any `create` call. The primary
    /// thread receives `PpuThreadId::PRIMARY` and starts
    /// `Runnable` with no exit value.
    ///
    /// Panics if a primary thread has already been inserted.
    pub fn insert_primary(&mut self, unit_id: UnitId, attrs: PpuThreadAttrs) {
        assert!(
            !self.threads.contains_key(&PpuThreadId::PRIMARY),
            "primary thread already inserted"
        );
        let thread = PpuThread {
            id: PpuThreadId::PRIMARY,
            unit_id,
            state: PpuThreadState::Runnable,
            attrs,
            exit_value: None,
            join_waiters: Vec::new(),
        };
        self.threads.insert(PpuThreadId::PRIMARY, thread);
        self.unit_to_thread.insert(unit_id, PpuThreadId::PRIMARY);
    }

    /// Create a new child thread and record its attributes.
    /// Returns the allocated id, or `None` if the id space is
    /// exhausted.
    pub fn create(&mut self, unit_id: UnitId, attrs: PpuThreadAttrs) -> Option<PpuThreadId> {
        let id = self.allocator.allocate()?;
        let thread = PpuThread {
            id,
            unit_id,
            state: PpuThreadState::Runnable,
            attrs,
            exit_value: None,
            join_waiters: Vec::new(),
        };
        self.threads.insert(id, thread);
        self.unit_to_thread.insert(unit_id, id);
        Some(id)
    }

    /// Look up a thread by id.
    pub fn get(&self, id: PpuThreadId) -> Option<&PpuThread> {
        self.threads.get(&id)
    }

    /// Mutably look up a thread by id.
    pub fn get_mut(&mut self, id: PpuThreadId) -> Option<&mut PpuThread> {
        self.threads.get_mut(&id)
    }

    /// Look up a thread by its runtime unit id.
    pub fn get_by_unit(&self, unit_id: UnitId) -> Option<&PpuThread> {
        self.unit_to_thread
            .get(&unit_id)
            .and_then(|id| self.threads.get(id))
    }

    /// Mutably look up a thread by its runtime unit id.
    pub fn get_by_unit_mut(&mut self, unit_id: UnitId) -> Option<&mut PpuThread> {
        let id = *self.unit_to_thread.get(&unit_id)?;
        self.threads.get_mut(&id)
    }

    /// Translate a runtime unit id to its guest thread id.
    pub fn thread_id_for_unit(&self, unit_id: UnitId) -> Option<PpuThreadId> {
        self.unit_to_thread.get(&unit_id).copied()
    }

    /// Mark a thread finished with the given exit value. Returns
    /// the list of threads that were waiting to join this one;
    /// callers are responsible for transitioning those units back
    /// to `Runnable` and clearing their block state.
    ///
    /// Returns an empty list if the thread does not exist.
    pub fn mark_finished(&mut self, id: PpuThreadId, exit_value: u64) -> Vec<PpuThreadId> {
        let thread = match self.threads.get_mut(&id) {
            Some(t) => t,
            None => return Vec::new(),
        };
        thread.state = PpuThreadState::Finished;
        thread.exit_value = Some(exit_value);
        std::mem::take(&mut thread.join_waiters)
    }

    /// Drain the list of joiners without changing thread state.
    /// Useful for tests and for the host to inspect pending
    /// joiners independently of the finish transition.
    pub fn drain_join_waiters(&mut self, id: PpuThreadId) -> Vec<PpuThreadId> {
        match self.threads.get_mut(&id) {
            Some(t) => std::mem::take(&mut t.join_waiters),
            None => Vec::new(),
        }
    }

    /// Append a waiter to the target's join list. Returns `true`
    /// if the target exists, `false` if not.
    pub fn add_join_waiter(&mut self, target: PpuThreadId, waiter: PpuThreadId) -> bool {
        match self.threads.get_mut(&target) {
            Some(t) => {
                t.join_waiters.push(waiter);
                true
            }
            None => false,
        }
    }

    /// Mark a thread `Detached`. Finished detached threads are
    /// garbage-collected without a join. Returns `true` if the
    /// target exists.
    pub fn detach(&mut self, id: PpuThreadId) -> bool {
        match self.threads.get_mut(&id) {
            Some(t) => {
                t.state = PpuThreadState::Detached;
                true
            }
            None => false,
        }
    }

    /// Number of threads in the table (including Finished /
    /// Detached that have not been purged).
    pub fn len(&self) -> usize {
        self.threads.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.threads.is_empty()
    }

    /// Iterate all thread ids in deterministic order.
    pub fn iter_ids(&self) -> impl Iterator<Item = PpuThreadId> + '_ {
        self.threads.keys().copied()
    }

    /// FNV-1a hash of the table for determinism checking. Folds
    /// id, unit_id, state, and exit value for every tracked thread
    /// in deterministic (BTreeMap) order. Attributes are omitted
    /// (they are immutable after create and mirrored in guest
    /// memory); join-waiter order is included because it affects
    /// wake order.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for (id, thread) in &self.threads {
            hasher.write(&id.raw().to_le_bytes());
            hasher.write(&thread.unit_id.raw().to_le_bytes());
            let state_byte = match &thread.state {
                PpuThreadState::Runnable => 0u8,
                PpuThreadState::Blocked(_) => 1,
                PpuThreadState::Finished => 2,
                PpuThreadState::Detached => 3,
            };
            hasher.write(&[state_byte]);
            if let PpuThreadState::Blocked(reason) = &thread.state {
                let (tag, payload) = match *reason {
                    GuestBlockReason::WaitingOnJoin { target } => {
                        let mut p = [0u8; 24];
                        p[0..8].copy_from_slice(&target.raw().to_le_bytes());
                        (0u8, p)
                    }
                    GuestBlockReason::WaitingOnLwMutex { id } => {
                        let mut p = [0u8; 24];
                        p[0..4].copy_from_slice(&id.to_le_bytes());
                        (1, p)
                    }
                    GuestBlockReason::WaitingOnMutex { id } => {
                        let mut p = [0u8; 24];
                        p[0..4].copy_from_slice(&id.to_le_bytes());
                        (2, p)
                    }
                    GuestBlockReason::WaitingOnSemaphore { id } => {
                        let mut p = [0u8; 24];
                        p[0..4].copy_from_slice(&id.to_le_bytes());
                        (3, p)
                    }
                    GuestBlockReason::WaitingOnEventQueue { id } => {
                        let mut p = [0u8; 24];
                        p[0..4].copy_from_slice(&id.to_le_bytes());
                        (4, p)
                    }
                    GuestBlockReason::WaitingOnEventFlag { id, mask, mode } => {
                        let mut p = [0u8; 24];
                        p[0..4].copy_from_slice(&id.to_le_bytes());
                        p[4..12].copy_from_slice(&mask.to_le_bytes());
                        p[12] = mode as u8;
                        (5, p)
                    }
                    GuestBlockReason::WaitingOnCond { cond_id, mutex_id } => {
                        let mut p = [0u8; 24];
                        p[0..4].copy_from_slice(&cond_id.to_le_bytes());
                        p[4..8].copy_from_slice(&mutex_id.to_le_bytes());
                        (6, p)
                    }
                };
                hasher.write(&[tag]);
                hasher.write(&payload);
            }
            if let Some(v) = thread.exit_value {
                hasher.write(&[1]);
                hasher.write(&v.to_le_bytes());
            } else {
                hasher.write(&[0]);
            }
            for waiter in &thread.join_waiters {
                hasher.write(&waiter.raw().to_le_bytes());
            }
        }
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_id_is_reserved_value() {
        assert_eq!(PpuThreadId::PRIMARY.raw(), 0x0100_0000);
    }

    #[test]
    fn fresh_allocator_starts_above_primary() {
        let mut a = PpuThreadIdAllocator::new();
        let first = a.allocate().unwrap();
        assert_eq!(first.raw(), 0x0100_0001);
        assert_ne!(first, PpuThreadId::PRIMARY);
    }

    #[test]
    fn allocator_is_monotonic() {
        let mut a = PpuThreadIdAllocator::new();
        let ids: Vec<_> = (0..8).map(|_| a.allocate().unwrap()).collect();
        for pair in ids.windows(2) {
            assert!(pair[0] < pair[1]);
        }
        assert_eq!(ids[0].raw(), 0x0100_0001);
        assert_eq!(ids[7].raw(), 0x0100_0008);
    }

    #[test]
    fn allocator_never_returns_primary() {
        // Even across thousands of allocations the primary id must
        // not surface.
        let mut a = PpuThreadIdAllocator::new();
        for _ in 0..10_000 {
            let id = a.allocate().unwrap();
            assert_ne!(id, PpuThreadId::PRIMARY);
        }
    }

    #[test]
    fn allocator_exhausts_on_u64_max() {
        // Pre-seed to the last allocatable slot so we can prove
        // exhaustion without allocating 2^64 times.
        let mut a = PpuThreadIdAllocator::with_next(u64::MAX - 1);
        let last = a.allocate().unwrap();
        assert_eq!(last.raw(), u64::MAX - 1);
        // Next call overflows -- the sentinel stays None.
        assert!(a.allocate().is_none());
        assert!(a.allocate().is_none());
    }

    #[test]
    fn peek_does_not_advance() {
        let mut a = PpuThreadIdAllocator::new();
        let peeked = a.peek().unwrap();
        let allocated = a.allocate().unwrap();
        assert_eq!(peeked, allocated);
        // A second peek sees the new next value.
        let peeked2 = a.peek().unwrap();
        let allocated2 = a.allocate().unwrap();
        assert_eq!(peeked2, allocated2);
        assert!(peeked < peeked2);
    }

    #[test]
    fn peek_returns_none_when_exhausted() {
        let mut a = PpuThreadIdAllocator::with_next(u64::MAX - 1);
        a.allocate().unwrap();
        // next is now u64::MAX; peek returns None (would overflow).
        assert!(a.peek().is_none());
    }

    #[test]
    fn two_fresh_allocators_produce_the_same_sequence() {
        // Determinism check: independent allocators with the same
        // seed produce byte-identical id sequences.
        let mut a = PpuThreadIdAllocator::new();
        let mut b = PpuThreadIdAllocator::new();
        for _ in 0..16 {
            assert_eq!(a.allocate(), b.allocate());
        }
    }

    // ---- PpuThreadTable tests ----

    fn dummy_attrs() -> PpuThreadAttrs {
        PpuThreadAttrs {
            entry: 0x10_0000,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x10000,
            priority: 1000,
            tls_base: 0x0020_0000,
        }
    }

    #[test]
    fn new_table_is_empty() {
        let t = PpuThreadTable::new();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn insert_primary_records_unit_mapping() {
        let mut t = PpuThreadTable::new();
        t.insert_primary(UnitId::new(1), dummy_attrs());
        assert_eq!(t.len(), 1);
        let p = t.get(PpuThreadId::PRIMARY).unwrap();
        assert_eq!(p.id, PpuThreadId::PRIMARY);
        assert_eq!(p.unit_id, UnitId::new(1));
        assert_eq!(p.state, PpuThreadState::Runnable);
        // Reverse lookup works.
        assert_eq!(
            t.thread_id_for_unit(UnitId::new(1)),
            Some(PpuThreadId::PRIMARY)
        );
    }

    #[test]
    #[should_panic(expected = "primary thread already inserted")]
    fn double_primary_insert_panics() {
        let mut t = PpuThreadTable::new();
        t.insert_primary(UnitId::new(1), dummy_attrs());
        t.insert_primary(UnitId::new(2), dummy_attrs());
    }

    #[test]
    fn create_allocates_above_primary() {
        let mut t = PpuThreadTable::new();
        t.insert_primary(UnitId::new(1), dummy_attrs());
        let c1 = t.create(UnitId::new(2), dummy_attrs()).unwrap();
        let c2 = t.create(UnitId::new(3), dummy_attrs()).unwrap();
        assert_eq!(c1.raw(), 0x0100_0001);
        assert_eq!(c2.raw(), 0x0100_0002);
        assert!(c1 > PpuThreadId::PRIMARY);
        assert!(c2 > c1);
    }

    #[test]
    fn create_records_unit_and_attrs() {
        let mut t = PpuThreadTable::new();
        let mut attrs = dummy_attrs();
        attrs.arg = 0xdead_beef;
        let id = t.create(UnitId::new(5), attrs.clone()).unwrap();
        let thread = t.get(id).unwrap();
        assert_eq!(thread.unit_id, UnitId::new(5));
        assert_eq!(thread.attrs.arg, 0xdead_beef);
        assert_eq!(thread.state, PpuThreadState::Runnable);
        assert!(thread.join_waiters.is_empty());
        assert!(thread.exit_value.is_none());
        // Reverse lookup.
        assert_eq!(t.get_by_unit(UnitId::new(5)).unwrap().id, id);
    }

    #[test]
    fn get_by_unit_unknown_returns_none() {
        let t = PpuThreadTable::new();
        assert!(t.get_by_unit(UnitId::new(99)).is_none());
    }

    #[test]
    fn mark_finished_sets_state_and_exit_value() {
        let mut t = PpuThreadTable::new();
        let id = t.create(UnitId::new(2), dummy_attrs()).unwrap();
        let waiters = t.mark_finished(id, 0x42);
        assert!(waiters.is_empty());
        let thread = t.get(id).unwrap();
        assert_eq!(thread.state, PpuThreadState::Finished);
        assert_eq!(thread.exit_value, Some(0x42));
    }

    #[test]
    fn mark_finished_unknown_returns_empty() {
        let mut t = PpuThreadTable::new();
        let waiters = t.mark_finished(PpuThreadId::new(0x9999), 0);
        assert!(waiters.is_empty());
    }

    #[test]
    fn add_join_waiter_and_mark_finished_drain_waiters() {
        let mut t = PpuThreadTable::new();
        t.insert_primary(UnitId::new(1), dummy_attrs());
        let child = t.create(UnitId::new(2), dummy_attrs()).unwrap();
        let third = t.create(UnitId::new(3), dummy_attrs()).unwrap();
        // Primary and third both join on child.
        assert!(t.add_join_waiter(child, PpuThreadId::PRIMARY));
        assert!(t.add_join_waiter(child, third));
        // Finish child -- both waiters come out in FIFO order.
        let waiters = t.mark_finished(child, 0);
        assert_eq!(waiters, vec![PpuThreadId::PRIMARY, third]);
        // After drain the child's list is empty.
        assert!(t.get(child).unwrap().join_waiters.is_empty());
    }

    #[test]
    fn add_join_waiter_unknown_target_returns_false() {
        let mut t = PpuThreadTable::new();
        assert!(!t.add_join_waiter(PpuThreadId::new(0x9999), PpuThreadId::PRIMARY));
    }

    #[test]
    fn drain_join_waiters_without_state_change() {
        let mut t = PpuThreadTable::new();
        let child = t.create(UnitId::new(2), dummy_attrs()).unwrap();
        t.add_join_waiter(child, PpuThreadId::PRIMARY);
        let waiters = t.drain_join_waiters(child);
        assert_eq!(waiters, vec![PpuThreadId::PRIMARY]);
        // State is unchanged -- still runnable.
        assert_eq!(t.get(child).unwrap().state, PpuThreadState::Runnable);
    }

    #[test]
    fn detach_sets_state() {
        let mut t = PpuThreadTable::new();
        let id = t.create(UnitId::new(2), dummy_attrs()).unwrap();
        assert!(t.detach(id));
        assert_eq!(t.get(id).unwrap().state, PpuThreadState::Detached);
    }

    #[test]
    fn detach_unknown_returns_false() {
        let mut t = PpuThreadTable::new();
        assert!(!t.detach(PpuThreadId::new(0x9999)));
    }

    #[test]
    fn blocked_state_carries_guest_reason() {
        let mut t = PpuThreadTable::new();
        let waiter = t.create(UnitId::new(2), dummy_attrs()).unwrap();
        let target = t.create(UnitId::new(3), dummy_attrs()).unwrap();
        // Park the waiter on a join for the target.
        t.get_mut(waiter).unwrap().state =
            PpuThreadState::Blocked(GuestBlockReason::WaitingOnJoin { target });
        match &t.get(waiter).unwrap().state {
            PpuThreadState::Blocked(GuestBlockReason::WaitingOnJoin { target: tgt }) => {
                assert_eq!(*tgt, target);
            }
            other => panic!("expected WaitingOnJoin, got {other:?}"),
        }
    }

    #[test]
    fn all_guest_block_reason_variants_round_trip_through_blocked_state() {
        let mut t = PpuThreadTable::new();
        let waiter = t.create(UnitId::new(2), dummy_attrs()).unwrap();
        let reasons = [
            GuestBlockReason::WaitingOnJoin {
                target: PpuThreadId::PRIMARY,
            },
            GuestBlockReason::WaitingOnLwMutex { id: 7 },
            GuestBlockReason::WaitingOnMutex { id: 7 },
            GuestBlockReason::WaitingOnSemaphore { id: 7 },
            GuestBlockReason::WaitingOnEventQueue { id: 7 },
            GuestBlockReason::WaitingOnEventFlag {
                id: 7,
                mask: 0xF0F0,
                mode: EventFlagWaitMode::AndClear,
            },
            GuestBlockReason::WaitingOnCond {
                cond_id: 7,
                mutex_id: 8,
            },
        ];
        for reason in reasons {
            t.get_mut(waiter).unwrap().state = PpuThreadState::Blocked(reason);
            match &t.get(waiter).unwrap().state {
                PpuThreadState::Blocked(stored) => assert_eq!(*stored, reason),
                other => panic!("expected Blocked, got {other:?}"),
            }
        }
    }

    #[test]
    fn state_hash_distinguishes_every_guest_block_reason() {
        // The state_hash tagging must give each variant a distinct
        // discriminant. Two tables differing only in block reason
        // must hash differently.
        fn table_with_reason(reason: GuestBlockReason) -> u64 {
            let mut t = PpuThreadTable::new();
            let id = t.create(UnitId::new(1), dummy_attrs()).unwrap();
            t.get_mut(id).unwrap().state = PpuThreadState::Blocked(reason);
            t.state_hash()
        }
        let hashes = [
            table_with_reason(GuestBlockReason::WaitingOnJoin {
                target: PpuThreadId::PRIMARY,
            }),
            table_with_reason(GuestBlockReason::WaitingOnLwMutex { id: 1 }),
            table_with_reason(GuestBlockReason::WaitingOnMutex { id: 1 }),
            table_with_reason(GuestBlockReason::WaitingOnSemaphore { id: 1 }),
            table_with_reason(GuestBlockReason::WaitingOnEventQueue { id: 1 }),
            table_with_reason(GuestBlockReason::WaitingOnEventFlag {
                id: 1,
                mask: 0,
                mode: EventFlagWaitMode::AndNoClear,
            }),
            table_with_reason(GuestBlockReason::WaitingOnCond {
                cond_id: 1,
                mutex_id: 1,
            }),
        ];
        for (i, h_i) in hashes.iter().enumerate() {
            for (j, h_j) in hashes.iter().enumerate().skip(i + 1) {
                assert_ne!(h_i, h_j, "variants {i} and {j} hash-collided");
            }
        }
    }

    #[test]
    fn state_hash_distinguishes_event_flag_wait_modes() {
        fn hash_with_mode(mode: EventFlagWaitMode) -> u64 {
            let mut t = PpuThreadTable::new();
            let id = t.create(UnitId::new(1), dummy_attrs()).unwrap();
            t.get_mut(id).unwrap().state =
                PpuThreadState::Blocked(GuestBlockReason::WaitingOnEventFlag {
                    id: 1,
                    mask: 0xAA,
                    mode,
                });
            t.state_hash()
        }
        let a = hash_with_mode(EventFlagWaitMode::AndNoClear);
        let b = hash_with_mode(EventFlagWaitMode::AndClear);
        let c = hash_with_mode(EventFlagWaitMode::OrNoClear);
        let d = hash_with_mode(EventFlagWaitMode::OrClear);
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
        assert_ne!(b, c);
        assert_ne!(b, d);
        assert_ne!(c, d);
    }

    #[test]
    fn state_hash_empty_table_is_stable() {
        let a = PpuThreadTable::new();
        let b = PpuThreadTable::new();
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_differs_when_thread_added() {
        let empty = PpuThreadTable::new();
        let mut populated = PpuThreadTable::new();
        populated.create(UnitId::new(1), dummy_attrs()).unwrap();
        assert_ne!(empty.state_hash(), populated.state_hash());
    }

    #[test]
    fn state_hash_changes_on_finish() {
        let mut a = PpuThreadTable::new();
        let mut b = PpuThreadTable::new();
        let id_a = a.create(UnitId::new(1), dummy_attrs()).unwrap();
        let id_b = b.create(UnitId::new(1), dummy_attrs()).unwrap();
        assert_eq!(a.state_hash(), b.state_hash());
        a.mark_finished(id_a, 42);
        assert_ne!(a.state_hash(), b.state_hash());
        b.mark_finished(id_b, 42);
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn tls_template_empty_is_recognizable() {
        let t = TlsTemplate::empty();
        assert!(t.is_empty());
        assert_eq!(t.mem_size(), 0);
        assert_eq!(t.align(), 0);
        assert_eq!(t.vaddr(), 0);
        assert!(t.initial_bytes().is_empty());
    }

    #[test]
    fn tls_template_stores_every_field() {
        let bytes = vec![0xAA, 0xBB, 0xCC];
        let t = TlsTemplate::new(bytes.clone(), 0x100, 0x10, 0x89_5cd0);
        assert_eq!(t.initial_bytes(), bytes.as_slice());
        assert_eq!(t.mem_size(), 0x100);
        assert_eq!(t.align(), 0x10);
        assert_eq!(t.vaddr(), 0x89_5cd0);
        assert!(!t.is_empty());
    }

    #[test]
    fn tls_template_hash_distinguishes_mutations() {
        let a = TlsTemplate::new(vec![1, 2, 3], 0x100, 0x10, 0x1000);
        let b = TlsTemplate::new(vec![1, 2, 3], 0x100, 0x10, 0x1000);
        assert_eq!(a.state_hash(), b.state_hash());
        // Different initial bytes -> different hash.
        let c = TlsTemplate::new(vec![1, 2, 4], 0x100, 0x10, 0x1000);
        assert_ne!(a.state_hash(), c.state_hash());
        // Different memsz -> different hash.
        let d = TlsTemplate::new(vec![1, 2, 3], 0x200, 0x10, 0x1000);
        assert_ne!(a.state_hash(), d.state_hash());
        // Different vaddr -> different hash.
        let e = TlsTemplate::new(vec![1, 2, 3], 0x100, 0x10, 0x2000);
        assert_ne!(a.state_hash(), e.state_hash());
    }

    #[test]
    fn tls_template_instantiate_copies_initial_bytes_and_zero_fills_tail() {
        let init = vec![0xAA, 0xBB, 0xCC, 0xDD];
        let t = TlsTemplate::new(init.clone(), 0x20, 0x10, 0x1000);
        let block = t.instantiate();
        assert_eq!(block.len(), 0x20);
        assert_eq!(&block[..4], init.as_slice());
        // Tail is zero-filled.
        assert!(block[4..].iter().all(|&b| b == 0));
    }

    #[test]
    fn tls_template_instantiate_produces_independent_blocks() {
        let t = TlsTemplate::new(vec![0x11, 0x22, 0x33], 0x10, 0x10, 0x1000);
        let mut a = t.instantiate();
        let b = t.instantiate();
        // Identical at start.
        assert_eq!(a, b);
        // Mutating `a` does not bleed into `b`.
        a[0] = 0xFF;
        a[5] = 0xAA;
        assert_ne!(a, b);
        assert_eq!(b[0], 0x11);
        assert_eq!(b[5], 0x00);
    }

    #[test]
    fn tls_template_instantiate_empty_template_is_empty_block() {
        let t = TlsTemplate::empty();
        assert!(t.instantiate().is_empty());
    }

    #[test]
    fn tls_template_instantiate_handles_filesz_eq_memsz() {
        // When filesz == memsz there is no BSS tail -- the block
        // is a pure copy of the template.
        let init = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let t = TlsTemplate::new(init.clone(), init.len() as u64, 0x10, 0x1000);
        assert_eq!(t.instantiate(), init);
    }

    #[test]
    fn stack_allocator_three_children_non_overlapping() {
        let mut a = ThreadStackAllocator::new();
        let s1 = a.allocate(0x10_000, 0x10).unwrap();
        let s2 = a.allocate(0x10_000, 0x10).unwrap();
        let s3 = a.allocate(0x10_000, 0x10).unwrap();
        // First allocation starts at the base.
        assert_eq!(s1.base, ThreadStackAllocator::CHILD_STACK_BASE);
        // Subsequent allocations are strictly above the previous
        // block's end, and the three ranges do not overlap.
        assert!(s2.base >= s1.end());
        assert!(s3.base >= s2.end());
        assert_ne!(s1.base, s2.base);
        assert_ne!(s2.base, s3.base);
        // All three are above the primary stack top
        // (0xD0010000 > 0xD000FFFF).
        assert!(s1.base > 0xD000_FFFF);
    }

    #[test]
    fn stack_allocator_is_deterministic_across_instances() {
        let mut a = ThreadStackAllocator::new();
        let mut b = ThreadStackAllocator::new();
        for _ in 0..4 {
            assert_eq!(a.allocate(0x10_000, 0x10), b.allocate(0x10_000, 0x10));
        }
    }

    #[test]
    fn stack_allocator_honors_alignment() {
        let mut a = ThreadStackAllocator::new();
        // Allocate an unaligned size first to push `next` off a
        // page boundary.
        let _ = a.allocate(0x4321, 0x10).unwrap();
        let s = a.allocate(0x1000, 0x1000).unwrap();
        assert_eq!(s.base & 0xFFF, 0, "base not 4KB-aligned");
    }

    #[test]
    fn stack_allocator_minimum_alignment_is_16_bytes() {
        let mut a = ThreadStackAllocator::new();
        let s = a.allocate(0x100, 0).unwrap();
        // Base must be 16-byte-aligned even when align=0 is
        // requested -- it is the PowerPC ABI minimum.
        assert_eq!(s.base & 0xF, 0);
    }

    #[test]
    fn thread_stack_initial_sp_leaves_16_byte_reserve() {
        let s = ThreadStack {
            base: 0xD001_0000,
            size: 0x10_000,
        };
        assert_eq!(s.initial_sp(), 0xD002_0000 - 0x10);
        assert_eq!(s.end(), 0xD002_0000);
    }

    #[test]
    fn stack_allocator_returns_none_on_overflow() {
        let mut a = ThreadStackAllocator {
            next: u64::MAX - 0x100,
        };
        assert!(a.allocate(0x1000, 0x10).is_none());
    }

    #[test]
    fn iter_ids_returns_deterministic_order() {
        let mut t = PpuThreadTable::new();
        t.insert_primary(UnitId::new(1), dummy_attrs());
        t.create(UnitId::new(2), dummy_attrs()).unwrap();
        t.create(UnitId::new(3), dummy_attrs()).unwrap();
        let ids: Vec<_> = t.iter_ids().collect();
        assert_eq!(ids.len(), 3);
        // BTreeMap iteration is sorted ascending.
        assert_eq!(ids[0], PpuThreadId::PRIMARY);
        assert_eq!(ids[1].raw(), 0x0100_0001);
        assert_eq!(ids[2].raw(), 0x0100_0002);
        // A second iteration returns the same order.
        let ids2: Vec<_> = t.iter_ids().collect();
        assert_eq!(ids, ids2);
    }
}
