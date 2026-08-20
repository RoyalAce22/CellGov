//! Process identity table.
//!
//! One entry per guest process. Boots start with exactly the boot
//! process; `sys_process_spawns_a_self2` inserts children.

use std::collections::BTreeMap;

use cellgov_ps3_abi::sys_process::{BOOT_PROCESS_PID, BOOT_PROCESS_PPID};

/// Per-process identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEntry {
    /// Parent pid.
    pub ppid: u32,
    /// SELF program authority id; the retail-application fallback
    /// for raw-ELF input. Served by `sys_ss_access_control_engine`
    /// pkg 2 and consulted by firmware to classify the caller.
    pub authority_id: u64,
    /// `ctrl_flags1` from the SELF's plaintext capability header;
    /// 0 without the record. Source of the root/debug predicates.
    pub control_flags1: u32,
    /// `Some(status)` once the process has exited. The entry stays
    /// in the table so `sys_process_get_status` polls resolve
    /// deterministically after the exit.
    pub exit_status: Option<i32>,
}

/// Table of guest processes, keyed by pid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessTable {
    entries: BTreeMap<u32, ProcessEntry>,
    /// Unit -> owning pid; absent means the boot process. Populated
    /// by the runtime after it registers a spawned child's units.
    unit_processes: BTreeMap<cellgov_event::UnitId, u32>,
}

impl ProcessTable {
    /// Table holding only the boot process with fallback identity.
    pub fn new_boot() -> Self {
        let mut entries = BTreeMap::new();
        entries.insert(
            BOOT_PROCESS_PID,
            ProcessEntry {
                ppid: BOOT_PROCESS_PPID,
                authority_id: cellgov_ps3_abi::sce::RETAIL_APP_PROGRAM_AUTHORITY_ID,
                control_flags1: 0,
                exit_status: None,
            },
        );
        Self {
            entries,
            unit_processes: BTreeMap::new(),
        }
    }

    /// The boot process entry.
    ///
    /// # Panics
    /// If the boot entry is absent; [`Self::remove_child`] refuses
    /// the boot pid, so this cannot fire.
    pub fn boot(&self) -> &ProcessEntry {
        self.entries
            .get(&BOOT_PROCESS_PID)
            .expect("boot process entry always present")
    }

    /// Mutable view of [`Self::boot`].
    pub fn boot_mut(&mut self) -> &mut ProcessEntry {
        self.entries
            .get_mut(&BOOT_PROCESS_PID)
            .expect("boot process entry always present")
    }

    /// Read-only lookup by pid.
    pub fn get(&self, pid: u32) -> Option<&ProcessEntry> {
        self.entries.get(&pid)
    }

    /// Insert a child process entry. Returns `false` (no overwrite)
    /// when the pid is already present.
    pub fn insert_child(&mut self, pid: u32, entry: ProcessEntry) -> bool {
        if self.entries.contains_key(&pid) {
            return false;
        }
        self.entries.insert(pid, entry);
        true
    }

    /// Number of table entries; exited children stay counted because
    /// their entries are retained for `sys_process_get_status`.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table is empty (never true in practice).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate `(pid, entry)` in pid order.
    pub fn iter(&self) -> impl Iterator<Item = (&u32, &ProcessEntry)> {
        self.entries.iter()
    }

    /// Mutable lookup by pid.
    pub fn get_mut(&mut self, pid: u32) -> Option<&mut ProcessEntry> {
        self.entries.get_mut(&pid)
    }

    /// Remove a child entry and its unit bindings; spawn-failure
    /// rollback. The boot entry is never removed.
    pub fn remove_child(&mut self, pid: u32) -> Option<ProcessEntry> {
        if pid == BOOT_PROCESS_PID {
            return None;
        }
        self.unit_processes.retain(|_, p| *p != pid);
        self.entries.remove(&pid)
    }

    /// Next unused child pid. LV2's real allocation pattern is not
    /// decoded; a deterministic `max + 0x100` in the boot pid's
    /// numbering style stands in until a capture pins it.
    pub fn next_child_pid(&self) -> u32 {
        let max = self
            .entries
            .keys()
            .next_back()
            .copied()
            .unwrap_or(BOOT_PROCESS_PID);
        max.saturating_add(0x100)
    }

    /// Bind `unit` to `pid`; untracked units are the boot process.
    pub fn bind_unit(&mut self, unit: cellgov_event::UnitId, pid: u32) {
        self.unit_processes.insert(unit, pid);
    }

    /// The pid `unit` belongs to.
    pub fn process_of_unit(&self, unit: cellgov_event::UnitId) -> u32 {
        self.unit_processes
            .get(&unit)
            .copied()
            .unwrap_or(BOOT_PROCESS_PID)
    }

    /// Units bound to `pid`, in unit-id order. Empty for the boot
    /// process (its units are never bound).
    pub fn units_of(&self, pid: u32) -> Vec<cellgov_event::UnitId> {
        self.unit_processes
            .iter()
            .filter(|(_, p)| **p == pid)
            .map(|(u, _)| *u)
            .collect()
    }

    /// Iterate `(unit, pid)` bindings in unit-id order.
    pub fn unit_bindings(&self) -> impl Iterator<Item = (&cellgov_event::UnitId, &u32)> {
        self.unit_processes.iter()
    }
}

#[cfg(test)]
#[path = "tests/table_tests.rs"]
mod tests;
