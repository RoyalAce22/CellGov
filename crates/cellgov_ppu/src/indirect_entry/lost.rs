//! The origins of a lost mark: the drops of `JoinFlow` whose constants a
//! lost register can hold.

use std::rc::Rc;

/// The most drop ids that one lost mark names.
///
/// A mark keeps the smallest ids of its origins. These are the smallest
/// ids of the union of every path's origins, so a join of capped marks
/// gives the same mark as a join of the full ones.
const ORIGIN_LIMIT: usize = 16;

/// The drops whose constants a lost register can hold.
///
/// A drop id is the index of the drop in the record of one `JoinFlow`
/// run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Origins {
    /// The smallest drop ids, in ascending order, at most
    /// [`ORIGIN_LIMIT`] of them.
    drops: Rc<[u32]>,
    /// Whether the register can also hold a constant of a drop that
    /// `drops` does not name.
    more: bool,
}

impl Origins {
    /// The origins of a register that the drop `drop` marks.
    pub(super) fn of(drop: u32) -> Self {
        Self {
            drops: Rc::from([drop]),
            more: false,
        }
    }

    /// The origins of a register that can hold a dropped constant of a
    /// drop that the record does not name.
    pub(super) fn unnamed() -> Self {
        Self {
            drops: Rc::from([]),
            more: true,
        }
    }

    /// The drop ids that the mark names, in ascending order.
    pub(super) fn drops(&self) -> &[u32] {
        &self.drops
    }

    /// Whether the register can also hold a constant of a drop that
    /// [`Self::drops`] does not name.
    pub(super) fn has_more(&self) -> bool {
        self.more
    }

    /// The origins of `self` and of `other`.
    pub(super) fn union(&self, other: &Self) -> Self {
        if self.holds_all(other) {
            return self.clone();
        }
        if other.holds_all(self) {
            return other.clone();
        }
        let mut drops: Vec<u32> = [&self.drops[..], &other.drops[..]].concat();
        drops.sort_unstable();
        drops.dedup();
        let more = self.more || other.more || drops.len() > ORIGIN_LIMIT;
        drops.truncate(ORIGIN_LIMIT);
        Self {
            drops: drops.into(),
            more,
        }
    }

    /// Whether the union of `self` and `other` is `self`.
    ///
    /// `self` holds a drop when it names the drop, or when it names
    /// [`ORIGIN_LIMIT`] drops and has more, and the drop is above each
    /// of them.
    fn holds_all(&self, other: &Self) -> bool {
        if !self.more && other.more {
            return false;
        }
        if Rc::ptr_eq(&self.drops, &other.drops) {
            return true;
        }
        let above = match self.drops.last() {
            Some(&last) if self.more && self.drops.len() == ORIGIN_LIMIT => last,
            _ => u32::MAX,
        };
        other
            .drops
            .iter()
            .all(|drop| *drop > above || self.drops.binary_search(drop).is_ok())
    }
}

/// Joins the origins `path` into `own`, and returns whether `own`
/// changed.
pub(super) fn join_origins(own: &mut Option<Origins>, path: Option<&Origins>) -> bool {
    let Some(path) = path else {
        return false;
    };
    let joined = match own {
        Some(own) if own.holds_all(path) => return false,
        Some(own) => own.union(path),
        None => path.clone(),
    };
    *own = Some(joined);
    true
}

/// The origins of each lost GPR, by GPR number in ascending order.
///
/// A GPR that the list does not hold is not lost.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct LostGprs(Vec<(u8, Origins)>);

impl LostGprs {
    /// The origins of `gpr`, or `None` when it is not lost.
    pub(super) fn get(&self, gpr: u8) -> Option<&Origins> {
        let at = self.0.binary_search_by_key(&gpr, |&(own, _)| own).ok()?;
        Some(&self.0[at].1)
    }

    /// Sets the origins of `gpr`. `None` clears its mark.
    pub(super) fn set(&mut self, gpr: u8, origins: Option<Origins>) {
        match (self.0.binary_search_by_key(&gpr, |&(own, _)| own), origins) {
            (Ok(at), Some(origins)) => self.0[at].1 = origins,
            (Ok(at), None) => {
                self.0.remove(at);
            }
            (Err(at), Some(origins)) => self.0.insert(at, (gpr, origins)),
            (Err(_), None) => {}
        }
    }

    /// Clears the mark of each GPR `n` whose bit `n` of `gprs` is set.
    pub(super) fn forget(&mut self, gprs: u32) {
        if gprs != 0 && !self.0.is_empty() {
            self.0
                .retain(|&(gpr, _)| gprs.checked_shr(u32::from(gpr)).unwrap_or(0) & 1 == 0);
        }
    }

    /// Joins `origins` into the origins of `gpr`, and returns whether
    /// they changed.
    pub(super) fn join_gpr(&mut self, gpr: u8, origins: &Origins) -> bool {
        let mut own = self.get(gpr).cloned();
        if !join_origins(&mut own, Some(origins)) {
            return false;
        }
        self.set(gpr, own);
        true
    }

    /// Joins the marks of `other` into `self`, and returns whether
    /// `self` changed.
    pub(super) fn join(&mut self, other: &Self) -> bool {
        let mut changed = false;
        for (gpr, origins) in &other.0 {
            changed |= self.join_gpr(*gpr, origins);
        }
        changed
    }
}
