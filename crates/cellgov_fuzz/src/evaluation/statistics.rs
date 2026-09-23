//! Order statistics and a rank-based comparison, in integers only.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// Five-number summary over sorted samples.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Distribution {
    /// Every sample, ascending.
    pub samples: Vec<u64>,
    /// Smallest sample.
    pub minimum: u64,
    /// Nearest-rank first quartile.
    pub lower_quartile: u64,
    /// Nearest-rank median.
    pub median: u64,
    /// Nearest-rank third quartile.
    pub upper_quartile: u64,
    /// Largest sample.
    pub maximum: u64,
    /// Sum of the samples, saturating.
    pub total: u64,
}

impl Distribution {
    /// Summarizes `samples`; `None` when there are none.
    #[must_use]
    pub fn of(samples: impl IntoIterator<Item = u64>) -> Option<Self> {
        let mut samples = samples.into_iter().collect::<Vec<_>>();
        samples.sort_unstable();
        let (&minimum, &maximum) = (samples.first()?, samples.last()?);
        Some(Self {
            minimum,
            lower_quartile: nearest_rank(&samples, 1, 4),
            median: nearest_rank(&samples, 1, 2),
            upper_quartile: nearest_rank(&samples, 3, 4),
            maximum,
            total: samples
                .iter()
                .fold(0u64, |total, sample| total.saturating_add(*sample)),
            samples,
        })
    }
}

/// The sample at rank `ceil(n * numerator / denominator)`, one-based.
fn nearest_rank(sorted: &[u64], numerator: usize, denominator: usize) -> u64 {
    let rank = (sorted.len() * numerator).div_ceil(denominator).max(1);
    sorted[rank - 1]
}

/// Effect size of a difference between two distributions, in conventional bands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Magnitude {
    /// Probability of superiority within 0.06 of one half: the distributions
    /// overlap almost entirely.
    Negligible,
    /// At least 0.06 from one half.
    Small,
    /// At least 0.14 from one half.
    Medium,
    /// At least 0.21 from one half.
    Large,
}

/// Exact probability that a candidate sample exceeds a baseline sample; a tie counts as half.
///
/// This is the Mann-Whitney U statistic over the number of sample pairs. It
/// compares two randomized outcomes and assumes no shape for either
/// distribution. [Klees2018 p:2128 s:Statistically Sound Comparisons]
/// The same ratio is the A12 effect size, so one number says both whether
/// the candidate tends to win and by how much.
/// [Klees2018 p:2129 s:Statistically Sound Comparisons]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Superiority {
    /// Twice the wins plus the ties.
    pub favourable: u64,
    /// Twice the number of pairs.
    pub pairs: u64,
}

impl Superiority {
    /// Compares every candidate sample with every baseline sample.
    ///
    /// `None` when either side has no sample.
    #[must_use]
    pub fn of(candidate: &[u64], baseline: &[u64]) -> Option<Self> {
        if candidate.is_empty() || baseline.is_empty() {
            return None;
        }
        let mut favourable = 0u64;
        for &left in candidate {
            for &right in baseline {
                favourable += match left.cmp(&right) {
                    Ordering::Greater => 2,
                    Ordering::Equal => 1,
                    Ordering::Less => 0,
                };
            }
        }
        Some(Self {
            favourable,
            pairs: 2 * candidate.len() as u64 * baseline.len() as u64,
        })
    }

    /// Which side the samples favour: greater when the candidate tends to
    /// exceed the baseline.
    #[must_use]
    pub fn favours(&self) -> Ordering {
        (2 * self.favourable).cmp(&self.pairs)
    }

    /// How far the probability sits from one half, on either side.
    #[must_use]
    pub fn magnitude(&self) -> Magnitude {
        // Distance from one half, scaled by 200 * pairs so the bands are
        // integer thresholds: |p - 0.5| >= 0.06, 0.14, 0.21.
        let doubled = 2 * self.favourable;
        let distance = doubled.abs_diff(self.pairs).saturating_mul(100);
        if distance >= 42 * self.pairs {
            Magnitude::Large
        } else if distance >= 28 * self.pairs {
            Magnitude::Medium
        } else if distance >= 12 * self.pairs {
            Magnitude::Small
        } else {
            Magnitude::Negligible
        }
    }
}
