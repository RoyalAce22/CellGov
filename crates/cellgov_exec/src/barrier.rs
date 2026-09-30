//! The barrier instructions a unit retires, named for the trace.
//!
//! A barrier orders storage accesses on hardware. This model executes
//! every unit's accesses in order and commits the effects of each batch
//! together, so no barrier changes a result here. A recompiler that runs
//! the same program on a host that reorders accesses needs the barrier's
//! location, so the runtime records each retired barrier by kind and
//! address and nothing else.

/// One barrier instruction's kind.
///
/// The discriminants equal the trace's wire encoding, so a new kind goes
/// at the end of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum BarrierKind {
    /// PPU `sync` (L=0), the heavyweight form.
    ///
    /// [PPC-Book2 p:26 s:3.3.3] the L field selects heavyweight sync, lwsync or ptesync.
    Sync = 0,
    /// PPU `lwsync` (sync L=1).
    Lwsync = 1,
    /// PPU `ptesync` (sync L=2).
    Ptesync = 2,
    /// PPU `sync` with the reserved L value 3.
    ///
    /// [PPC-Book2 p:26 s:3.3.3] L=3 is reserved; the decoder accepts it and this kind names it.
    SyncL3 = 3,
    /// PPU `eieio`.
    ///
    /// [PPC-Book2 p:28 s:3.3.3] eieio orders caching-inhibited guarded accesses and, as a separate set, stores to ordinary coherent storage.
    Eieio = 4,
    /// PPU `isync`.
    ///
    /// [PPC-Book2 p:22 s:3.3.1] isync waits for earlier instructions to complete and discards prefetched ones.
    Isync = 5,
    /// SPU `sync`.
    ///
    /// [SPU-ISA p:242 s:10] sync completes pending stores before the next instruction fetch.
    SpuSync = 6,
    /// SPU `sync.c`, which also synchronizes channel writes.
    ///
    /// [SPU-ISA p:258 s:13.9] only sync.c guarantees that a channel write to execution state affects the next instruction.
    SpuSyncC = 7,
    /// SPU `dsync`.
    ///
    /// [SPU-ISA p:243 s:10] dsync completes earlier loads, stores and channel accesses before later ones start.
    SpuDsync = 8,
}

impl BarrierKind {
    /// Every kind, in discriminant order.
    pub const VARIANTS: [Self; 9] = [
        Self::Sync,
        Self::Lwsync,
        Self::Ptesync,
        Self::SyncL3,
        Self::Eieio,
        Self::Isync,
        Self::SpuSync,
        Self::SpuSyncC,
        Self::SpuDsync,
    ];

    /// The PPU `sync` kind for L field `l`; only the low two bits count.
    pub const fn ppu_sync(l: u8) -> Self {
        match l & 3 {
            0 => Self::Sync,
            1 => Self::Lwsync,
            2 => Self::Ptesync,
            _ => Self::SyncL3,
        }
    }
}

/// One barrier instruction a unit retired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RetiredBarrier {
    /// The guest address of the barrier instruction.
    pub pc: u64,
    /// Which barrier it is.
    pub kind: BarrierKind,
}
