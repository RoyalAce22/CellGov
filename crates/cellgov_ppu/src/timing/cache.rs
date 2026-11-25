//! Tag-only models of the PPE's L1 data cache and L2 cache.
//!
//! [CBE-Handbook p:137 s:6.1.3.8] the L1 data cache replaces by a
//! 4-way pseudo-LRU policy, updated whenever a line is read; it never
//! casts out data, because it writes through.
//!
//! [CBE-Handbook p:139 s:6.1.5] the L2 is inclusive of the L1 data
//! cache, write-back, and allocates on a store miss.

use super::op::Tail;
use cellgov_ps3_abi::hw::ppu_issue as k;
use cellgov_ps3_abi::hw::ppu_mmu::SMALL_PAGE_LOG2;

#[cfg(test)]
#[path = "tests/cancelled_touch_tests.rs"]
mod cancelled_touch_tests;
#[cfg(test)]
#[path = "tests/reload_queue_tests.rs"]
mod reload_queue_tests;

/// Where a load found its line. A later variant is farther from the core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Level {
    L1,
    L2,
    Memory,
}

/// The `len` bytes a load or store reaches from effective address `ea`
/// and real address `ra`.
///
/// When `tail` is `Some`, the bytes from its offset on are at its real
/// address. Their effective addresses follow `ea` without a gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Operand {
    pub(crate) ea: u64,
    pub(crate) ra: u64,
    pub(crate) len: u8,
    pub(crate) tail: Option<Tail>,
}

#[cfg(test)]
impl Operand {
    /// The `len` bytes at `ea` on a thread that does not translate.
    pub(crate) const fn untranslated(ea: u64, len: u8) -> Self {
        Self {
            ea,
            ra: ea,
            len,
            tail: None,
        }
    }
}

/// One line an access reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Line {
    /// The line number of the effective address, which picks the L1 set.
    index: u64,
    /// The line number of the real address, which tags the line in both
    /// caches and picks its L2 set.
    real: u64,
}

impl Line {
    fn of(ea: u64, ra: u64) -> Self {
        Self {
            index: ea / k::PPU_CACHE_LINE_BYTES,
            real: ra / k::PPU_CACHE_LINE_BYTES,
        }
    }
}

/// A set-associative tag array with tree pseudo-LRU replacement.
///
/// [CBE-Handbook p:135 s:6.1] the L1 data cache has an effective-addressed
/// index and real-addressed tags, so two distinct lines never share a tag.
/// Each way keeps its whole line number for this reason.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Tags<const SETS: usize, const WAYS: usize> {
    /// Each way's line number. Only a way that `valid` marks holds a line.
    lines: [[u64; WAYS]; SETS],
    /// Each set's ways that hold a line, bit `w` for way `w`.
    valid: [u8; SETS],
    /// Each set's tree: bit `n - 1` is node `n` of a heap-ordered tree,
    /// and points to the half that holds the next victim.
    tree: [u8; SETS],
    /// Whether each way holds data newer than memory.
    modified: [[bool; WAYS]; SETS],
}

impl<const SETS: usize, const WAYS: usize> Tags<SETS, WAYS> {
    const LEVELS: u32 = WAYS.trailing_zeros();

    fn new() -> Self {
        const {
            assert!(
                WAYS <= u8::BITS as usize,
                "a set's valid byte marks each way"
            )
        };
        Self {
            lines: [[0; WAYS]; SETS],
            valid: [0; SETS],
            tree: [0; SETS],
            modified: [[false; WAYS]; SETS],
        }
    }

    fn set_of(line: u64) -> usize {
        (line % SETS as u64) as usize
    }

    fn holds(&self, set: usize, way: usize) -> bool {
        self.valid[set] >> way & 1 != 0
    }

    fn way_of(&self, set: usize, line: u64) -> Option<usize> {
        (0..WAYS).find(|&w| self.holds(set, w) && self.lines[set][w] == line)
    }

    fn find(&self, line: u64) -> Option<(usize, usize)> {
        let set = Self::set_of(line);
        self.way_of(set, line).map(|w| (set, w))
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

    /// Whether `set` holds `line`; a hit updates the tree.
    fn lookup_in(&mut self, set: usize, line: u64) -> bool {
        match self.way_of(set, line) {
            Some(way) => {
                self.touch(set, way);
                true
            }
            None => false,
        }
    }

    /// Whether `line` is present; a hit updates the tree.
    fn lookup(&mut self, line: u64) -> bool {
        self.lookup_in(Self::set_of(line), line)
    }

    /// Place `line` in `set`, and return the line it evicted.
    fn fill_in(&mut self, set: usize, line: u64) -> Option<u64> {
        let empty = (!self.valid[set]).trailing_zeros() as usize;
        let way = if empty < WAYS {
            empty
        } else {
            self.victim(set)
        };
        let old = self.holds(set, way).then_some(self.lines[set][way]);
        self.lines[set][way] = line;
        self.valid[set] |= 1 << way;
        self.modified[set][way] = false;
        self.touch(set, way);
        old
    }

    /// Place `line`, and return the line it evicted.
    fn fill(&mut self, line: u64) -> Option<u64> {
        self.fill_in(Self::set_of(line), line)
    }

    fn drop_way(&mut self, set: usize, way: usize) {
        self.valid[set] &= !(1 << way);
        self.modified[set][way] = false;
    }

    fn invalidate(&mut self, line: u64) {
        if let Some((set, way)) = self.find(line) {
            self.drop_way(set, way);
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

/// The lines of a small page.
const PAGE_LINES: usize = (1 << SMALL_PAGE_LOG2) / k::PPU_CACHE_LINE_BYTES as usize;

/// The L1 data cache: the effective line picks the set, and the real line
/// is the tag.
///
/// [CBE-Handbook p:137 s:6.1.3.9] The L1 data cache is indexed with effective addresses.
///
/// An effective address and its real address agree in the offset of a
/// small page. So a real line can sit only in the sets that agree with
/// it in the line bits of that offset: two of the 64 sets. The L1 holds
/// at most one copy of a real line.
///
/// [CBE-Handbook p:137 s:6.1.3.10] Data that two effective addresses alias can be in any of four ways at each of two congruence classes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct L1 {
    tags: Tags<{ k::PPU_L1D_SETS }, { k::PPU_L1D_WAYS }>,
}

impl L1 {
    fn new() -> Self {
        const {
            assert!(
                k::PPU_L1D_SETS.is_multiple_of(PAGE_LINES),
                "a page offset's line bits pick the low bits of a set"
            )
        };
        Self { tags: Tags::new() }
    }

    /// The set that the effective line of `line` picks.
    fn set_of(line: Line) -> usize {
        (line.index % k::PPU_L1D_SETS as u64) as usize
    }

    /// The set and the way that hold real line `real`, in any set that
    /// can hold it.
    fn find(&self, real: u64) -> Option<(usize, usize)> {
        let low = (real % PAGE_LINES as u64) as usize;
        (low..k::PPU_L1D_SETS)
            .step_by(PAGE_LINES)
            .find_map(|set| self.tags.way_of(set, real).map(|way| (set, way)))
    }

    /// Whether the set of `line`'s effective line holds its real line; a
    /// hit updates the tree. A copy in another set is no hit.
    fn lookup(&mut self, line: Line) -> bool {
        self.tags.lookup_in(Self::set_of(line), line.real)
    }

    /// Place `line` in the set of its effective line.
    ///
    /// The L1 first drops the copy of its real line that another set holds.
    ///
    /// [CBE-Handbook p:138 s:6.1.3.10] A line found at the other congruence class is invalidated, and the data from the L2 is placed at the congruence class of the effective address.
    fn fill(&mut self, line: Line) {
        self.invalidate(line.real);
        self.tags.fill_in(Self::set_of(line), line.real);
    }

    /// Drop real line `real` from the set that holds it.
    fn invalidate(&mut self, real: u64) {
        if let Some((set, way)) = self.find(real) {
            self.tags.drop_way(set, way);
        }
    }

    /// Drop the copy of `line`'s real line that a set other than its
    /// effective line's set holds.
    fn drop_alias(&mut self, line: Line) {
        if let Some((set, way)) = self.find(line.real) {
            if set != Self::set_of(line) {
                self.tags.drop_way(set, way);
            }
        }
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

    /// Delays each cycle later than `began` by `stalled`.
    fn stretch(&mut self, began: u64, stalled: u64) {
        for entry in &mut self.entries {
            if entry.0 != 0 && entry.1 > began {
                entry.1 += stalled;
                self.until = self.until.max(entry.1);
            }
        }
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
    l1: L1,
    l2: Tags<{ k::PPU_L2_SETS }, { k::PPU_L2_WAYS }>,
    /// The cycle each touched real line arrives.
    touched: LineCycles,
    /// The first cycle the next touch can send its request.
    touch_free: u64,
    /// The first cycle each L1 set can take the next reload.
    ///
    /// A touch that reloads a line holds its L1 set until its trip ends,
    /// plus [`k::PPU_LOAD_MISS_ENTRY_REUSE_CYCLES`]. CellGov takes the
    /// trip as the cycle the reload's last beat returns. The evidence is
    /// the `ppu_issue_touch_set_pressure` capture, in cycles a load:
    /// - 1, 2, 4 and 8 touches of L2-held lines in the load's L1 set add
    ///   49, 96, 190 and 370 to a chase through memory;
    /// - 8 such touches spread over 8 sets add 70.
    ///
    /// The same capture has 6 touches in the set add 367, about what 8
    /// add. One hold for each touch gives 6 touches about 290. CellGov does not
    /// model why 6 cost as much as 8.
    ///
    /// A load that misses the L1, an `ldarx` included, waits for its set
    /// and then holds it the same way. In cycles a load of the chase:
    /// - `ppu_issue_touch_set_forms`: a load of an L2-held line in the
    ///   chase's set adds what one touch there adds, and a load and then
    ///   a touch add what two touches add. An `ldarx` after 4 touches in
    ///   its set takes 182 more than one after none.
    /// - `ppu_issue_touch_set_holds`: a load of a line from memory in the
    ///   chase's set adds 408, and an `ldarx` of an L2-held line there
    ///   adds 77. A load from memory in another set adds 410, which
    ///   CellGov does not model, so the 408 does not tell the set's hold
    ///   from that wait.
    reload_free: [u64; k::PPU_L1D_SETS],
    /// The cycle each touched real line's L1 set let its request go, for
    /// a touch that waited for its set.
    held: LineCycles,
    /// The cycle each flush or store-back finishes with its real line.
    busy: LineCycles,
}

impl Caches {
    pub(crate) fn new() -> Self {
        Self {
            l1: L1::new(),
            l2: Tags::new(),
            touched: LineCycles::new(),
            touch_free: 0,
            reload_free: [0; k::PPU_L1D_SETS],
            held: LineCycles::new(),
            busy: LineCycles::new(),
        }
    }

    fn line(ra: u64) -> u64 {
        ra / k::PPU_CACHE_LINE_BYTES
    }

    /// The line of the first byte and the line of the last byte of `at`.
    ///
    /// When `at.tail` is `Some`, the last byte's real address is on the
    /// tail's page. An access of no bytes covers the line at its address.
    /// In the `ppu_issue_line_cross_waits` capture, an `lswx` with a byte
    /// count of 0 brings the line at its address into the L1.
    ///
    /// [CBE-Handbook p:135 s:6.1] The L1 data cache has real-addressed tags.
    ///
    /// [CBE-Handbook p:140 s:6.1.5.1] The L2 has a real-address index and real-address tags.
    fn lines(at: Operand) -> (Line, Line) {
        let span = u64::from(at.len.max(1)) - 1;
        let (last_ea, last_ra) = match at.tail {
            Some(t) => (
                at.ea.wrapping_add(span),
                t.ra.saturating_add(span.saturating_sub(u64::from(t.at))),
            ),
            None => {
                // The real address stops at 2^64, and the effective address
                // stops with it. On a thread that does not translate, the
                // last byte's effective line then equals its real line.
                let span = span.min(u64::MAX - at.ra);
                (at.ea.wrapping_add(span), at.ra + span)
            }
        };
        (Line::of(at.ea, at.ra), Line::of(last_ea, last_ra))
    }

    /// Allocate real line `real` in the L2, and drop from the L1 the line
    /// that the L2 evicts.
    ///
    /// [CBE-Handbook p:140 s:6.1.5.1] The L2 includes all of the L1 data cache contents.
    ///
    /// The documents do not say how the L2 finds the L1 set of the line
    /// it evicts. The inclusion fixes the result: the line leaves the set
    /// that holds it.
    fn fill_l2(&mut self, real: u64) {
        if let Some(evicted) = self.l2.fill(real) {
            self.l1.invalidate(evicted);
        }
    }

    /// A load at cycle `t` of the bytes of `at`. It returns:
    /// - the level of the farthest line it reads;
    /// - the first cycle its L1 misses can go to the L2, or 0.
    ///
    /// `skip_l1` misses the L1 and leaves it as it was.
    ///
    /// In the `ppu_issue_line_cross` capture, an `ld` or `lmw` whose bytes
    /// reach a flushed line waits for that line from memory. With the
    /// second line in the L1, an `lmw` across two lines costs what it
    /// costs on one line.
    pub(crate) fn load_queued(&mut self, t: u64, at: Operand, skip_l1: bool) -> (Level, u64) {
        let (first, last) = Self::lines(at);
        let (level, queued) = self.load_line(t, first, skip_l1);
        if last == first {
            (level, queued)
        } else {
            let (last_level, last_queued) = self.load_line(t, last, skip_l1);
            (level.max(last_level), queued.max(last_queued))
        }
    }

    /// A load at cycle `t` of `line`, and the first cycle its L1 miss can
    /// go to the L2.
    ///
    /// A copy of the line in the L1 set of another effective line is no
    /// hit. The L2 then gives the line, because it includes the L1.
    ///
    /// The first of these that applies gives the cycle:
    /// - a load of a line that a touch still brings starts no earlier
    ///   than that touch's request;
    /// - an L1 hit waits for no set;
    /// - an L1 miss, or a load that skips the L1, waits for the reloads
    ///   that its L1 set holds, and then holds the set as a touch does.
    fn load_line(&mut self, t: u64, line: Line, skip_l1: bool) -> (Level, u64) {
        let in_flight = !skip_l1 && self.touched.after(t, line.real) > t;
        let held = if in_flight {
            self.held.after(t, line.real)
        } else {
            0
        };
        if !skip_l1 && self.l1.lookup(line) {
            return (Level::L1, held);
        }
        let set = L1::set_of(line);
        let queued = if in_flight {
            held
        } else {
            self.reload_free[set]
        };
        let level = if self.l2.lookup(line.real) {
            Level::L2
        } else {
            self.fill_l2(line.real);
            // [CBE-Handbook p:146 s:6.1.6.3] the L2 ends a prefetch when a
            // demand load of the same address is ahead of it.
            self.touched.forget(line.real);
            self.held.forget(line.real);
            Level::Memory
        };
        if !in_flight {
            let trip = match level {
                Level::Memory => k::PPU_MEMORY_LOAD_USE_CYCLES,
                Level::L1 | Level::L2 => k::PPU_L2_LOAD_USE_CYCLES,
            };
            self.reload_free[set] = t.max(queued) + trip + k::PPU_LOAD_MISS_ENTRY_REUSE_CYCLES;
        }
        if !skip_l1 {
            self.l1.fill(line);
        }
        (level, queued)
    }

    /// A store of the bytes of `at`: the L1 keeps its copy of each line
    /// current, and the L2 allocates each line it misses.
    pub(crate) fn store(&mut self, at: Operand) {
        let (first, last) = Self::lines(at);
        self.store_line(first);
        if last != first {
            self.store_line(last);
        }
    }

    /// A store to `line`. The L1 drops a copy of the line in the set of
    /// another effective line, and places no line.
    ///
    /// [CBE-Handbook p:135 s:6.1.3.1] The L1 data cache does not allocate on a write.
    ///
    /// The documents do not say what a store does with that copy.
    /// [CBE-Handbook p:138 s:6.1.3.10] invalidates the copy in the other
    /// congruence class on an access. CellGov applies that rule to a store,
    /// without the reload.
    fn store_line(&mut self, line: Line) {
        self.l1.drop_alias(line);
        if !self.l2.lookup(line.real) {
            self.fill_l2(line.real);
        }
        self.l2.modify(line.real);
    }

    /// `dcbz`: drop the line at real address `ra` from the L1 and allocate
    /// it in the L2.
    ///
    /// [CBE-Handbook p:723 s:A.1.2] `dcbz` invalidates the L1 line on its
    /// way to the L2; allocation and zeroing occur at the L2.
    ///
    /// [CBE-Handbook p:147 s:6.1.6.4] When the line that `dcbz` addresses is present in the L1, the L1 invalidates it before the operation goes to the L2.
    pub(crate) fn zero(&mut self, ra: u64) {
        let line = Self::line(ra);
        self.l1.invalidate(line);
        if !self.l2.lookup(line) {
            self.fill_l2(line);
        }
        self.l2.modify(line);
    }

    /// A `dcbt` or `dcbtst` at cycle `t`: reload the line at real address
    /// `ra` into the L2, and into the L1 set of effective address `ea`.
    /// Remember the cycle it arrives.
    ///
    /// A cancelled touch (`ra` is `None`) changes nothing:
    /// - it fills no L1 or L2 line;
    /// - it sends no request, so the next touch can send its request as
    ///   early as before;
    /// - it records no arrival.
    ///
    /// [CBE-Handbook p:145 s:6.1.6.2] a touch acts like a load from the
    /// viewpoint of the cache hierarchy; with TH = '0000' the L1 is
    /// reloaded as well as the L2.
    ///
    /// [CBE-Handbook p:145 s:6.1.6.2] A touch that the PPE cancels does not reload the cache.
    ///
    /// [CBE-Handbook p:146 s:6.1.6.2] one cache line is requested at a
    /// time, from the L2 or from memory.
    ///
    /// The touch's request waits for the reloads that the L1 set of `ea`
    /// holds, and its own reload then holds that set. A touch of a line
    /// that an earlier touch still brings waits for no set and holds none.
    pub(crate) fn touch(&mut self, t: u64, ea: u64, ra: Option<u64>) {
        let Some(ra) = ra else {
            return;
        };
        let line = Line::of(ea, ra);
        if self.l1.lookup(line) {
            return;
        }
        let slot = t.max(self.touch_free);
        let (interval, trip) = if self.l2.lookup(line.real) {
            (k::PPU_TOUCH_L2_INTERVAL_CYCLES, k::PPU_L2_LOAD_USE_CYCLES)
        } else {
            self.fill_l2(line.real);
            (k::PPU_TOUCH_INTERVAL_CYCLES, k::PPU_MEMORY_LOAD_USE_CYCLES)
        };
        self.touch_free = slot + interval;
        let earlier = self.touched.after(t, line.real);
        let set = L1::set_of(line);
        let sent = if earlier > t {
            slot
        } else {
            let sent = slot.max(self.reload_free[set]);
            self.reload_free[set] = sent + trip + k::PPU_LOAD_MISS_ENTRY_REUSE_CYCLES;
            if sent > slot {
                self.held.record(t, line.real, sent);
            }
            sent
        };
        // [CBE-Handbook p:138 s:6.1.3.10] A copy in the other congruence class is reloaded from the L2, so it arrives no earlier than an earlier touch brings the line there.
        let arrival = (sent + trip).max(earlier);
        self.l1.fill(line);
        self.touched.record(t, line.real, arrival);
    }

    /// The latest cycle a touched line of the bytes of `at` arrives.
    ///
    /// The result is 0 when no touch of those lines is on its way at
    /// cycle `t`. A result at or before `t` means the lines are present.
    pub(crate) fn arrival(&self, t: u64, at: Operand) -> u64 {
        let (first, last) = Self::lines(at);
        self.touched
            .after(t, first.real)
            .max(self.touched.after(t, last.real))
    }

    /// The latest cycle a flush or store-back finishes with a line of the
    /// bytes of `at`.
    ///
    /// The result is 0 when no flush or store-back of those lines is under
    /// way at cycle `t`. A result at or before `t` means the lines are free.
    ///
    /// [CBE-Handbook p:147 s:6.1.6.5] `dcbf` and `dcbst` can complete
    /// before the operation they cause completes.
    pub(crate) fn busy(&self, t: u64, at: Operand) -> u64 {
        let (first, last) = Self::lines(at);
        self.busy
            .after(t, first.real)
            .max(self.busy.after(t, last.real))
    }

    /// Delays by `stalled` each cycle later than `began` of:
    /// - a touched line's arrival;
    /// - the cycle a touched line's L1 set let its request go;
    /// - an L1 set's next free reload.
    pub(crate) fn stretch_touches(&mut self, began: u64, stalled: u64) {
        self.touched.stretch(began, stalled);
        self.held.stretch(began, stalled);
        for free in &mut self.reload_free {
            if *free > began {
                *free += stalled;
            }
        }
    }

    /// A `dcbf` at cycle `t`: drop the line at real address `ra` from the
    /// L1 and the L2.
    ///
    /// [CBE-Handbook p:147 s:6.1.6.5] `dcbf` writes a modified line back
    /// and invalidates it in all caches.
    pub(crate) fn flush(&mut self, t: u64, ra: u64) {
        let line = Self::line(ra);
        let cycles = if self.l2.is_modified(line) {
            k::PPU_DCBF_MODIFIED_LINE_BUSY_CYCLES
        } else {
            k::PPU_DCBF_LINE_BUSY_CYCLES
        };
        self.l1.invalidate(line);
        self.l2.invalidate(line);
        self.touched.forget(line);
        self.held.forget(line);
        self.busy.record(t, line, t + cycles);
    }

    /// A `dcbst` at cycle `t`: the line at real address `ra` stays valid.
    ///
    /// [CBE-Handbook p:147 s:6.1.6.5] `dcbst` writes a modified line back
    /// and leaves it valid.
    ///
    /// [CBE-Handbook p:138 s:6.1.3.11] The L1 data cache never holds modified data, so a `dcbst` leaves the L1 as it was.
    pub(crate) fn store_back(&mut self, t: u64, ra: u64) {
        let line = Self::line(ra);
        let cycles = if self.l2.write_back(line) {
            k::PPU_DCBST_MODIFIED_LINE_BUSY_CYCLES
        } else {
            k::PPU_DCBST_LINE_BUSY_CYCLES
        };
        self.busy.record(t, line, t + cycles);
    }
}

#[cfg(test)]
impl Caches {
    /// Whether the L1 set of `ea`'s line holds the real line of `ra`, and
    /// whether the L2 holds that real line. The query moves no tree.
    pub(crate) fn holds(&self, ea: u64, ra: u64) -> (bool, bool) {
        let line = Line::of(ea, ra);
        let l1 = self.l1.tags.way_of(L1::set_of(line), line.real).is_some();
        (l1, self.l2.find(line.real).is_some())
    }

    /// The first cycle the next touch can send its request.
    pub(crate) fn touch_free(&self) -> u64 {
        self.touch_free
    }

    /// [`Self::load_queued`] at cycle 0, without the cycle its misses can
    /// start.
    pub(crate) fn load(&mut self, at: Operand, skip_l1: bool) -> Level {
        self.load_queued(0, at, skip_l1).0
    }

    /// The first cycle the L1 set of `ea`'s line can take the next reload.
    pub(crate) fn reload_free(&self, ea: u64) -> u64 {
        self.reload_free[L1::set_of(Line::of(ea, ea))]
    }
}
