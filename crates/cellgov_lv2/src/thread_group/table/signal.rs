//! The `SPU_Cfg` signal configuration and the kept
//! signal-notification values of a thread.

use super::ThreadGroupTable;
use crate::thread_group::GroupState;
use cellgov_event::UnitId;

impl ThreadGroupTable {
    /// The signal configuration of an initialized thread: the running
    /// SPU's while its group runs, the kept one otherwise.
    pub fn signal_config(&self, thread_id: u32) -> Option<u64> {
        let (group_id, slot) = self.slot_of_thread(thread_id)?;
        let group = self.groups.get(group_id)?;
        let s = group.slots.get(&slot)?;
        Some(match group.state {
            GroupState::Running => s.live_signal_config.unwrap_or(s.signal_config),
            _ => s.signal_config,
        })
    }

    /// Set the signal configuration of an initialized thread: the
    /// running SPU's while its group runs, the kept one otherwise.
    /// `false` when no initialized thread has the id.
    pub fn set_signal_config(&mut self, thread_id: u32, value: u64) -> bool {
        let Some((group_id, slot)) = self.slot_of_thread(thread_id) else {
            return false;
        };
        let Some(mut group) = self.groups.get_mut(group_id) else {
            return false;
        };
        let running = group.state == GroupState::Running;
        match group.slots.get_mut(&slot) {
            Some(s) if running => {
                s.live_signal_config = Some(value);
                true
            }
            Some(s) => {
                s.signal_config = value;
                true
            }
            None => false,
        }
    }

    /// Keep `value` for signal-notification register `register`, 0 or 1,
    /// of an initialized thread whose group does not run, in the mode its
    /// kept configuration gives that register. `false` when no initialized
    /// thread has the id or `register` names no register.
    pub fn save_signal(&mut self, thread_id: u32, register: u8, value: u32) -> bool {
        let Some((group_id, slot)) = self.slot_of_thread(thread_id) else {
            return false;
        };
        let Some(mut group) = self.groups.get_mut(group_id) else {
            return false;
        };
        let Some(s) = group.slots.get_mut(&slot) else {
            return false;
        };
        let index = usize::from(register);
        let Some(kept) = s.saved_signals.get_mut(index) else {
            return false;
        };
        let or_mode = s.signal_config >> index & 1 == 1;
        *kept = Some(match *kept {
            Some(old) if or_mode => old | value,
            _ => value,
        });
        true
    }

    /// The signal configuration of the thread a registered SPU runs,
    /// finished or not.
    pub fn signal_config_of_unit(&self, unit_id: UnitId) -> Option<u64> {
        let thread_id = self
            .thread_id_to_unit
            .iter()
            .find(|(_, &raw)| raw == unit_id.raw())
            .map(|(tid, _)| tid)?;
        self.signal_config(thread_id)
    }

    /// The signal-notification values the kernel keeps for the thread a
    /// registered SPU runs, register 1 first; a start loads the ones
    /// present into the SPU.
    pub fn saved_signals_of_unit(&self, unit_id: UnitId) -> [Option<u32>; 2] {
        let Some(thread_id) = self
            .thread_id_to_unit
            .iter()
            .find(|(_, &raw)| raw == unit_id.raw())
            .map(|(tid, _)| tid)
        else {
            return [None; 2];
        };
        self.slot_of_thread(thread_id)
            .and_then(|(group_id, slot)| {
                self.groups
                    .get(group_id)?
                    .slots
                    .get(&slot)
                    .map(|s| s.saved_signals)
            })
            .unwrap_or([None; 2])
    }
}
