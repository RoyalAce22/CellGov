//! Tag-only models of the PPE's L1 data cache and L2 cache.
//!
//! [CBE-Handbook p:137 s:6.1.3.8] the L1 data cache replaces by a
//! 4-way pseudo-LRU policy, updated whenever a line is read; it never
//! casts out data, because it writes through.
//!
//! [CBE-Handbook p:139 s:6.1.5] the L2 is inclusive of the L1 data
//! cache, write-back, and allocates on a store miss.

use cellgov_ps3_abi::hw::ppu_issue as k;

/// Where a load found its line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    L1,
    L2,
    Memory,
}

/// A set-associative tag array with tree pseudo-LRU replacement.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Tags<const SETS: usize, const WAYS: usize> {
    /// Each way's line number plus one; zero is an empty way.
    lines: [[u32; WAYS]; SETS],
    /// Each set's tree: bit `n - 1` is node `n` of a heap-ordered tree,
    /// and points to the half that holds the next victim.
    tree: [u8; SETS],
    /// Whether each way holds data newer than memory.
    modified: [[bool; WAYS]; SETS],
}

impl<const SETS: usize, const WAYS: usize> Tags<SETS, WAYS> {
    const LEVELS: u32 = WAYS.trailing_zeros();

    fn new() -> Self {
        Self {
            lines: [[0; WAYS]; SETS],
            tree: [0; SETS],
            modified: [[false; WAYS]; SETS],
        }
    }

    fn slot(line: u64) -> (usize, u32) {
        let set = (line % SETS as u64) as usize;
        (set, (line as u32).wrapping_add(1))
    }

    fn find(&self, line: u64) -> Option<(usize, usize)> {
        let (set, tag) = Self::slot(line);
        self.lines[set]
            .iter()
            .position(|w| *w == tag)
            .map(|w| (set, w))
    }

    /// Point the set's tree away from `way`.
    fn touch(&mut self, set: usize, way: usize) {
        let mut node = 1usize;
        for level in (0..Self::LEVELS).rev() {
            let side = (way >> level) & 1;
            let bit = 1u8 << (node - 1);
            if side == 0 {
                self.tree[set] |= bit;
            } else {
                self.tree[set] &= !bit;
            }
            node = 2 * node + side;
        }
    }

    fn victim(&self, set: usize) -> usize {
        let mut node = 1usize;
        let mut way = 0usize;
        for _ in 0..Self::LEVELS {
            let side = usize::from(self.tree[set] >> (node - 1) & 1);
            way = 2 * way + side;
            node = 2 * node + side;
        }
        way
    }

    /// Whether `line` is present; a hit updates the tree.
    fn lookup(&mut self, line: u64) -> bool {
        match self.find(line) {
            Some((set, way)) => {
                self.touch(set, way);
                true
            }
            None => false,
        }
    }

    /// Place `line`, and return the line it evicted.
    fn fill(&mut self, line: u64) -> Option<u64> {
        let (set, tag) = Self::slot(line);
        let way = self.lines[set]
            .iter()
            .position(|w| *w == 0)
            .unwrap_or_else(|| self.victim(set));
        let old = self.lines[set][way];
        self.lines[set][way] = tag;
        self.modified[set][way] = false;
        self.touch(set, way);
        (old != 0).then(|| u64::from(old - 1))
    }

    fn invalidate(&mut self, line: u64) {
        if let Some((set, way)) = self.find(line) {
            self.lines[set][way] = 0;
            self.modified[set][way] = false;
        }
    }

    /// Mark `line` as newer than memory, when present.
    fn modify(&mut self, line: u64) {
        if let Some((set, way)) = self.find(line) {
            self.modified[set][way] = true;
        }
    }

    /// Clears the modified mark of `line` and returns whether it was set.
    fn write_back(&mut self, line: u64) -> bool {
        match self.find(line) {
            Some((set, way)) => std::mem::take(&mut self.modified[set][way]),
            None => false,
        }
    }

    fn is_modified(&self, line: u64) -> bool {
        self.find(line)
            .is_some_and(|(set, way)| self.modified[set][way])
    }
}

/// How many lines a [`LineCycles`] remembers.
///
/// In the `ppu_issue_touch_forms` capture, a chase sends two touches to
/// memory for each load and loses none of them. The model has at most 17
/// touches outstanding there. The PPE's limit is not known; CellGov uses 32.
const LINE_CYCLES: usize = 32;

/// The most recent lines with an operation still under way, each with the
/// cycle that operation completes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LineCycles {
    /// Each line's number plus one, with its cycle; zero is an empty entry.
    entries: [(u64, u64); LINE_CYCLES],
    /// The latest cycle in `entries`.
    until: u64,
}

impl LineCycles {
    fn new() -> Self {
        Self {
            entries: [(0, 0); LINE_CYCLES],
            until: 0,
        }
    }

    /// Records `cycle` for `line` at cycle `t`.
    ///
    /// The entry it uses is the first of:
    /// - the line's own entry;
    /// - an empty entry, or one that finishes by `t`;
    /// - the entry that finishes first.
    fn record(&mut self, t: u64, line: u64, cycle: u64) {
        let own = self.entries.iter().position(|(tag, _)| *tag == line + 1);
        let slot = own
            .or_else(|| {
                self.entries
                    .iter()
                    .position(|(tag, done)| *tag == 0 || *done <= t)
            })
            .unwrap_or_else(|| {
                (0..LINE_CYCLES)
                    .min_by_key(|i| self.entries[*i].1)
                    .unwrap_or(0)
            });
        self.entries[slot] = (line + 1, cycle);
        self.until = self.until.max(cycle);
    }

    /// The latest cycle recorded for `line`, or 0 when no entry finishes
    /// after `t`. The result can be at or before `t`.
    fn after(&self, t: u64, line: u64) -> u64 {
        if t >= self.until {
            return 0;
        }
        self.entries
            .iter()
            .filter(|(tag, _)| *tag == line + 1)
            .map(|(_, cycle)| *cycle)
            .max()
            .unwrap_or(0)
    }

    fn forget(&mut self, line: u64) {
        for entry in &mut self.entries {
            if entry.0 == line + 1 {
                *entry = (0, 0);
            }
        }
    }
}

/// The L1 data cache and the L2, by 128-byte line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Caches {
    l1: Tags<{ k::PPU_L1D_SETS }, { k::PPU_L1D_WAYS }>,
    l2: Tags<{ k::PPU_L2_SETS }, { k::PPU_L2_WAYS }>,
    /// The cycle each touched line arrives.
    touched: LineCycles,
    /// The first cycle the next touch can send its request.
    touch_free: u64,
    /// The cycle each flush or store-back finishes with its line.
    busy: LineCycles,
}

impl Caches {
    pub(crate) fn new() -> Self {
        Self {
            l1: Tags::new(),
            l2: Tags::new(),
            touched: LineCycles::new(),
            touch_free: 0,
            busy: LineCycles::new(),
        }
    }

    fn line(ea: u64) -> u64 {
        ea / k::PPU_CACHE_LINE_BYTES
    }

    /// Allocate `line` in the L2, and drop from the L1 the line that the
    /// L2 evicts.
    fn fill_l2(&mut self, line: u64) {
        if let Some(evicted) = self.l2.fill(line) {
            self.l1.invalidate(evicted);
        }
    }

    /// A load of the line at `ea`. `skip_l1` misses the L1 and leaves it
    /// as it was.
    pub(crate) fn load(&mut self, ea: u64, skip_l1: bool) -> Level {
        let line = Self::line(ea);
        if !skip_l1 && self.l1.lookup(line) {
            return Level::L1;
        }
        let level = if self.l2.lookup(line) {
            Level::L2
        } else {
            self.fill_l2(line);
            // [CBE-Handbook p:146 s:6.1.6.3] the L2 ends a prefetch when a
            // demand load of the same address is ahead of it.
            self.touched.forget(line);
            Level::Memory
        };
        if !skip_l1 {
            self.l1.fill(line);
        }
        level
    }

    /// A store to the line at `ea`: the L1 keeps its copy current, and
    /// the L2 allocates on a miss.
    pub(crate) fn store(&mut self, ea: u64) {
        let line = Self::line(ea);
        if !self.l2.lookup(line) {
            self.fill_l2(line);
        }
        self.l2.modify(line);
    }

    /// `dcbz`: drop the line from the L1 and allocate it in the L2.
    ///
    /// [CBE-Handbook p:723 s:A.1.2] `dcbz` invalidates the L1 line on its
    /// way to the L2; allocation and zeroing occur at the L2.
    pub(crate) fn zero(&mut self, ea: u64) {
        let line = Self::line(ea);
        self.l1.invalidate(line);
        if !self.l2.lookup(line) {
            self.fill_l2(line);
        }
        self.l2.modify(line);
    }

    /// A `dcbt` or `dcbtst` at cycle `t`: reload the line at `ea` into the
    /// L2 and the L1, and remember the cycle it arrives.
    ///
    /// [CBE-Handbook p:145 s:6.1.6.2] a touch acts like a load from the
    /// viewpoint of the cache hierarchy; with TH = '0000' the L1 is
    /// reloaded as well as the L2.
    ///
    /// [CBE-Handbook p:146 s:6.1.6.2] one cache line is requested at a
    /// time, from the L2 or from memory.
    pub(crate) fn touch(&mut self, t: u64, ea: u64) {
        let line = Self::line(ea);
        if self.l1.lookup(line) {
            return;
        }
        let sent = t.max(self.touch_free);
        let (interval, trip) = if self.l2.lookup(line) {
            (k::PPU_TOUCH_L2_INTERVAL_CYCLES, k::PPU_L2_LOAD_USE_CYCLES)
        } else {
            self.fill_l2(line);
            (k::PPU_TOUCH_INTERVAL_CYCLES, k::PPU_MEMORY_LOAD_USE_CYCLES)
        };
        self.touch_free = sent + interval;
        let arrival = sent + trip;
        self.l1.fill(line);
        self.touched.record(t, line, arrival);
    }

    /// The cycle the touched line at `ea` arrives, or 0 when no touch is
    /// on its way at cycle `t`. A result at or before `t` means the line
    /// is present.
    pub(crate) fn arrival(&self, t: u64, ea: u64) -> u64 {
        self.touched.after(t, Self::line(ea))
    }

    /// The cycle a flush or store-back of the line at `ea` finishes with
    /// it, or 0 when no flush or store-back is under way at cycle `t`. A
    /// result at or before `t` means the line is free.
    ///
    /// [CBE-Handbook p:147 s:6.1.6.5] `dcbf` and `dcbst` can complete
    /// before the operation they cause completes.
    pub(crate) fn busy(&self, t: u64, ea: u64) -> u64 {
        self.busy.after(t, Self::line(ea))
    }

    /// A `dcbf` at cycle `t`: drop the line at `ea` from the L1 and the
    /// L2.
    ///
    /// [CBE-Handbook p:147 s:6.1.6.5] `dcbf` writes a modified line back
    /// and invalidates it in all caches.
    pub(crate) fn flush(&mut self, t: u64, ea: u64) {
        let line = Self::line(ea);
        let cycles = if self.l2.is_modified(line) {
            k::PPU_DCBF_MODIFIED_LINE_BUSY_CYCLES
        } else {
            k::PPU_DCBF_LINE_BUSY_CYCLES
        };
        self.l1.invalidate(line);
        self.l2.invalidate(line);
        self.touched.forget(line);
        self.busy.record(t, line, t + cycles);
    }

    /// A `dcbst` at cycle `t`: the line at `ea` stays valid.
    ///
    /// [CBE-Handbook p:147 s:6.1.6.5] `dcbst` writes a modified line back
    /// and leaves it valid.
    pub(crate) fn store_back(&mut self, t: u64, ea: u64) {
        let line = Self::line(ea);
        let cycles = if self.l2.write_back(line) {
            k::PPU_DCBST_MODIFIED_LINE_BUSY_CYCLES
        } else {
            k::PPU_DCBST_LINE_BUSY_CYCLES
        };
        self.busy.record(t, line, t + cycles);
    }
}
