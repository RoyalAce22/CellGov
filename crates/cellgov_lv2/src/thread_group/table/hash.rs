//! The table's partial of the sync-state sum.

use super::ThreadGroupTable;
use cellgov_mem::lanes::{self, source};

impl ThreadGroupTable {
    /// The table's partial of the sync-state sum: the groups, the three
    /// unit maps and the id allocator's cursor.
    pub fn sync_partial(&self) -> u128 {
        self.groups
            .partial()
            .wrapping_add(self.unit_to_group.partial())
            .wrapping_add(self.finished_units.partial())
            .wrapping_add(self.thread_id_to_unit.partial())
            .wrapping_add(self.raw_limits.partial())
            .wrapping_add(self.ids_term())
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub fn sync_partial_from_scratch(&self) -> u128 {
        self.groups
            .partial_from_scratch()
            .wrapping_add(self.unit_to_group.partial_from_scratch())
            .wrapping_add(self.finished_units.partial_from_scratch())
            .wrapping_add(self.thread_id_to_unit.partial_from_scratch())
            .wrapping_add(self.raw_limits.partial_from_scratch())
            .wrapping_add(self.ids_term())
    }

    /// The id allocator's term.
    fn ids_term(&self) -> u128 {
        lanes::value_term(source::GROUP_IDS, 0, &self.next_id)
    }
}
