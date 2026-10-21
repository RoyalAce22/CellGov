//! The PPE's branch prediction for one hardware thread: the branch
//! history table, its global history, and the link stack.
//!
//! [CBE-Handbook p:139 s:6.1.4] each thread has a 4096 x 2-bit branch
//! history table with 6 bits of global history, and a 4-entry link stack
//! that predicts the target of branch-to-link instructions. Only a
//! branch whose direction came from the table updates it.

use cellgov_ps3_abi::hw::ppu_issue as k;

/// Branch-history and link-stack state.
#[derive(Clone)]
pub(crate) struct Predictor {
    /// Two-bit counters: 2 and 3 predict taken.
    table: [u8; k::PPU_BHT_ENTRIES],
    /// Outcomes of the latest table-predicted branches, newest in bit 0.
    history: u32,
    link: [u64; k::PPU_LINK_STACK_ENTRIES],
    /// Entries pushed and not popped, at most the stack's size.
    depth: usize,
    /// The slot the next push writes.
    top: usize,
}

impl Predictor {
    pub(crate) fn new() -> Self {
        Self {
            table: [1; k::PPU_BHT_ENTRIES],
            history: 0,
            link: [0; k::PPU_LINK_STACK_ENTRIES],
            depth: 0,
            top: 0,
        }
    }

    /// Predict the branch at `pc` from the table, then train the table
    /// and the history on `taken`. Returns the prediction.
    ///
    /// The table index combines the branch's word address with the
    /// global history, as a gshare predictor does.
    pub(crate) fn predict(&mut self, pc: u64, taken: bool) -> bool {
        let mask = k::PPU_BHT_ENTRIES as u64 - 1;
        let index = (((pc >> 2) ^ u64::from(self.history)) & mask) as usize;
        let counter = self.table[index];
        self.table[index] = if taken {
            (counter + 1).min(3)
        } else {
            counter.saturating_sub(1)
        };
        let width = (1u32 << k::PPU_GLOBAL_HISTORY_BITS) - 1;
        self.history = ((self.history << 1) | u32::from(taken)) & width;
        counter >= 2
    }

    /// Record the return address of a branch that set LR.
    pub(crate) fn push(&mut self, ret: u64) {
        self.link[self.top] = ret;
        self.top = (self.top + 1) % k::PPU_LINK_STACK_ENTRIES;
        self.depth = (self.depth + 1).min(k::PPU_LINK_STACK_ENTRIES);
    }

    /// The predicted target of a branch to LR, or `None` when the stack
    /// is empty.
    pub(crate) fn pop(&mut self) -> Option<u64> {
        if self.depth == 0 {
            return None;
        }
        self.depth -= 1;
        self.top = (self.top + k::PPU_LINK_STACK_ENTRIES - 1) % k::PPU_LINK_STACK_ENTRIES;
        Some(self.link[self.top])
    }
}
