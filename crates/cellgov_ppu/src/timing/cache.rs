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
#[derive(Clone)]
struct Tags<const SETS: usize, const WAYS: usize> {
    /// Each way's line number plus one; zero is an empty way.
    lines: [[u32; WAYS]; SETS],
    /// Each set's tree: bit `n - 1` is node `n` of a heap-ordered tree,
    /// and points to the half that holds the next victim.
    tree: [u8; SETS],
}

impl<const SETS: usize, const WAYS: usize> Tags<SETS, WAYS> {
    const LEVELS: u32 = WAYS.trailing_zeros();

    fn new() -> Self {
        Self {
            lines: [[0; WAYS]; SETS],
            tree: [0; SETS],
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
        self.touch(set, way);
        (old != 0).then(|| u64::from(old - 1))
    }

    fn invalidate(&mut self, line: u64) {
        if let Some((set, way)) = self.find(line) {
            self.lines[set][way] = 0;
        }
    }
}

/// The L1 data cache and the L2, by 128-byte line.
#[derive(Clone)]
pub(crate) struct Caches {
    l1: Tags<{ k::PPU_L1D_SETS }, { k::PPU_L1D_WAYS }>,
    l2: Tags<{ k::PPU_L2_SETS }, { k::PPU_L2_WAYS }>,
}

impl Caches {
    pub(crate) fn new() -> Self {
        Self {
            l1: Tags::new(),
            l2: Tags::new(),
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
    }
}
