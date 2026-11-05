//! The load-path profile: per load site, whether the store buffer answers
//! a load and which region view holds it.
//!
//! It measures whether a per-site region cache would pay. A profiled unit
//! records each retired D-form or DS-form load and each load half of
//! `lwz; cmpwi`, `lwz; mtlr` and `ld; mtlr`. The address of the shadow
//! slot that holds the load keys its record.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// What one load's address resolved to before the load ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadOutcome {
    /// The store buffer held every byte, so no region view was read.
    Forwarded,
    /// The region view at this index of the step's views held every byte.
    Region(usize),
    /// No single region view held every byte; the load faults or reads
    /// across two views.
    Unheld,
}

/// One load site's history.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Site {
    /// Loads that read a region view.
    region_loads: u64,
    /// Region loads whose view was the one this site's previous region
    /// load read.
    same_as_previous: u64,
    /// The view this site's last region load read.
    previous: Option<usize>,
    /// Each view this site read, once.
    regions: Vec<usize>,
}

/// The load-path counts of one unit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadProfile {
    /// Loads the store buffer answered whole.
    pub forwarded: u64,
    /// Loads no single region view held.
    pub unheld: u64,
    /// Region loads that a unit with no shadow slot for their address
    /// ran.
    pub no_shadow: u64,
    /// Retired fused pairs with a load half the profile does not key.
    pub unkeyed_fused: u64,
    /// Batches whose region views differed from the previous batch's.
    pub layout_changes: u64,
    /// The `(base, length)` of each view at the last batch.
    layout: Option<Vec<(u64, u64)>>,
    /// The region-load history per shadow slot address.
    sites: BTreeMap<u64, Site>,
}

impl LoadProfile {
    /// Counts one retired load at the shadow slot address `slot`.
    ///
    /// `slot` is `None` when the unit has no shadow slot for the load.
    pub fn record(&mut self, slot: Option<u64>, outcome: LoadOutcome) {
        match outcome {
            LoadOutcome::Forwarded => self.forwarded += 1,
            LoadOutcome::Unheld => self.unheld += 1,
            LoadOutcome::Region(index) => {
                let Some(slot) = slot else {
                    self.no_shadow += 1;
                    return;
                };
                let site = self.sites.entry(slot).or_default();
                site.region_loads += 1;
                if site.previous == Some(index) {
                    site.same_as_previous += 1;
                }
                site.previous = Some(index);
                if !site.regions.contains(&index) {
                    site.regions.push(index);
                }
            }
        }
    }

    /// Counts a change of region layout when `views` differ from the
    /// views of the last batch this profile saw.
    pub fn note_layout(&mut self, views: impl Iterator<Item = (u64, u64)>) {
        let views: Vec<(u64, u64)> = views.collect();
        if self.layout.as_ref().is_some_and(|last| *last != views) {
            self.layout_changes += 1;
        }
        self.layout = Some(views);
    }

    /// Each keyed site's slot address and region loads, in address order.
    pub fn sites(&self) -> impl Iterator<Item = (u64, u64)> + '_ {
        self.sites
            .iter()
            .map(|(slot, site)| (*slot, site.region_loads))
    }

    /// Collects the totals and sums the per-site counts into one
    /// [`LoadSummary`].
    pub fn summary(&self) -> LoadSummary {
        let mut summary = LoadSummary {
            forwarded: self.forwarded,
            unheld: self.unheld,
            no_shadow: self.no_shadow,
            unkeyed_fused: self.unkeyed_fused,
            layout_changes: self.layout_changes,
            sites: self.sites.len() as u64,
            ..LoadSummary::default()
        };
        for site in self.sites.values() {
            summary.region_loads += site.region_loads;
            summary.same_as_previous += site.same_as_previous;
            let bucket = site.regions.len().clamp(1, 3) - 1;
            summary.sites_by_regions[bucket] += 1;
            summary.loads_by_regions[bucket] += site.region_loads;
        }
        summary
    }
}

/// The counts a [`LoadProfile`] reports, summed over units by
/// [`LoadSummary::merge`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoadSummary {
    /// Loads the store buffer answered whole.
    pub forwarded: u64,
    /// Loads that read a region view at a keyed site.
    pub region_loads: u64,
    /// Region loads that read the view their site's previous region load
    /// read.
    pub same_as_previous: u64,
    /// Loads no single region view held.
    pub unheld: u64,
    /// Region loads on a unit with no shadow slot for their address.
    pub no_shadow: u64,
    /// Retired fused pairs with a load half the profile does not key.
    pub unkeyed_fused: u64,
    /// Batches whose region views differed from the previous batch's.
    pub layout_changes: u64,
    /// Keyed load sites.
    pub sites: u64,
    /// Sites that read 1, 2, and 3 or more distinct views.
    pub sites_by_regions: [u64; 3],
    /// Region loads at the sites of each `sites_by_regions` bucket.
    pub loads_by_regions: [u64; 3],
}

impl LoadSummary {
    /// Adds `other`'s counts to this summary's.
    pub fn merge(&mut self, other: &Self) {
        self.forwarded += other.forwarded;
        self.region_loads += other.region_loads;
        self.same_as_previous += other.same_as_previous;
        self.unheld += other.unheld;
        self.no_shadow += other.no_shadow;
        self.unkeyed_fused += other.unkeyed_fused;
        self.layout_changes += other.layout_changes;
        self.sites += other.sites;
        for i in 0..3 {
            self.sites_by_regions[i] += other.sites_by_regions[i];
            self.loads_by_regions[i] += other.loads_by_regions[i];
        }
    }

    /// The summary as one JSON object, with each count under its field's
    /// name.
    pub fn to_json(&self) -> String {
        let mut out = String::from("{\n");
        let fields = [
            ("forwarded", self.forwarded),
            ("region_loads", self.region_loads),
            ("same_as_previous", self.same_as_previous),
            ("unheld", self.unheld),
            ("no_shadow", self.no_shadow),
            ("unkeyed_fused", self.unkeyed_fused),
            ("layout_changes", self.layout_changes),
            ("sites", self.sites),
        ];
        for (name, value) in fields {
            let _ = writeln!(out, "  \"{name}\": {value},");
        }
        let [s1, s2, s3] = self.sites_by_regions;
        let [l1, l2, l3] = self.loads_by_regions;
        let _ = writeln!(out, "  \"sites_by_regions\": [{s1}, {s2}, {s3}],");
        let _ = writeln!(out, "  \"loads_by_regions\": [{l1}, {l2}, {l3}]");
        out.push_str("}\n");
        out
    }
}
