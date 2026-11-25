//! The region views a batch runs against, and the windows its accesses
//! reach.

use super::exit::Attempt;
use crate::unit::ppu_unit::PpuExecutionUnit;
use cellgov_effects::Effect;

/// The batch's region views, on the stack while they fit.
///
/// The stack table avoids a heap allocation on the Budget=1 hot path.
/// Boot installs six regions, each shared-memory mapping adds one and
/// each window adds up to two; a larger layout spills to the heap.
/// Nothing guest-visible turns on the cutoff.
pub(super) struct RegionTable<'a> {
    storage: [cellgov_mem::RegionView<'a>; MAX_REGIONS],
    spill: Vec<cellgov_mem::RegionView<'a>>,
    len: usize,
}

/// How many region views [`RegionTable`] holds before it spills.
const MAX_REGIONS: usize = 8;

impl<'a> RegionTable<'a> {
    #[inline]
    pub(super) fn new(views: impl Iterator<Item = cellgov_mem::RegionView<'a>>) -> Self {
        let mut table = Self {
            storage: [cellgov_mem::RegionView::plain(0, &[]); MAX_REGIONS],
            spill: Vec::new(),
            len: 0,
        };
        for view in views {
            if table.len < MAX_REGIONS {
                table.storage[table.len] = view;
            } else {
                if table.spill.is_empty() {
                    table.spill.extend_from_slice(&table.storage);
                }
                table.spill.push(view);
            }
            table.len += 1;
        }
        table
    }

    #[inline]
    pub(super) fn as_slice(&self) -> &[cellgov_mem::RegionView<'a>] {
        if self.len <= MAX_REGIONS {
            &self.storage[..self.len]
        } else {
            &self.spill
        }
    }
}

impl PpuExecutionUnit {
    /// Whether the read intents and buffered stores `attempt` added reach a window.
    ///
    /// `span` holds every window of `mem`. A translating thread's stores
    /// reach a window at their real addresses.
    #[inline(always)]
    pub(super) fn reaches_window(
        &self,
        span: (u64, u64),
        mem: &cellgov_mem::GuestMemory,
        effects: &[Effect],
        attempt: &Attempt,
        translating: bool,
    ) -> bool {
        let new_effects = effects.get(attempt.effects_before..).unwrap_or_default();
        let stores = self.store_buf.stores_from(attempt.stores_before);
        if translating {
            let real = stores.flat_map(|(ea, len)| self.real_store_pieces(ea, len));
            touches_window(span, mem, new_effects, real)
        } else {
            touches_window(span, mem, new_effects, stores)
        }
    }
}

/// Whether one instruction's new read intents or staged stores reach a
/// window of `mem`.
///
/// `[start, end)` holds every window, so the function reads the window
/// list only for an access inside it.
fn touches_window(
    (start, end): (u64, u64),
    mem: &cellgov_mem::GuestMemory,
    new_effects: &[Effect],
    mut new_stores: impl Iterator<Item = (u64, u8)>,
) -> bool {
    let overlaps = |addr: u64, len: u64| {
        let last = addr.saturating_add(len);
        addr < end && last > start && mem.window_ranges().any(|(lo, hi)| addr < hi && last > lo)
    };
    new_effects.iter().any(|e| match e {
        Effect::SharedReadIntent { range, .. } => overlaps(range.start().raw(), range.length()),
        _ => false,
    }) || new_stores.any(|(addr, len)| overlaps(addr, u64::from(len)))
}
