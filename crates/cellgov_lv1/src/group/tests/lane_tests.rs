use core::ops::RangeInclusive;

use cellgov_mem::lanes::source::HYPERVISOR;

use super::*;

/// One group's lane sources as the guard reads them.
#[derive(Debug, Clone, Copy)]
struct Lanes<'a> {
    group: &'a str,
    ranges: &'a [RangeInclusive<u8>],
    /// Each hashed surface's name and lane source.
    sources: &'a [(&'a str, u8)],
}

/// A rule the groups' lane sources break.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LaneFault<'a> {
    /// A group's range reaches past the hypervisor's reserved tags.
    OutsideReservation {
        group: &'a str,
        range: RangeInclusive<u8>,
    },
    /// Two groups' ranges share a tag; the first tag they share.
    Overlap {
        first: &'a str,
        second: &'a str,
        tag: u8,
    },
    /// A hashed surface's lane source is outside its group's ranges.
    OutsideGroup {
        group: &'a str,
        surface: &'a str,
        source: u8,
    },
}

/// The rules `groups` break against the `reserved` tags: ranges outside
/// the reservation, then overlaps by group order, then sources outside
/// their group.
fn lane_faults<'a>(groups: &[Lanes<'a>], reserved: &[RangeInclusive<u8>]) -> Vec<LaneFault<'a>> {
    let mut faults = Vec::new();
    for lanes in groups {
        for range in lanes.ranges {
            let inside = reserved
                .iter()
                .any(|r| r.start() <= range.start() && range.end() <= r.end());
            if !inside {
                faults.push(LaneFault::OutsideReservation {
                    group: lanes.group,
                    range: range.clone(),
                });
            }
        }
    }
    for (index, first) in groups.iter().enumerate() {
        for second in &groups[index + 1..] {
            let shared = first.ranges.iter().find_map(|a| {
                second.ranges.iter().find_map(|b| {
                    let tag = *a.start().max(b.start());
                    (tag <= *a.end().min(b.end())).then_some(tag)
                })
            });
            if let Some(tag) = shared {
                faults.push(LaneFault::Overlap {
                    first: first.group,
                    second: second.group,
                    tag,
                });
            }
        }
    }
    for lanes in groups {
        for &(surface, source) in lanes.sources {
            if !lanes.ranges.iter().any(|range| range.contains(&source)) {
                faults.push(LaneFault::OutsideGroup {
                    group: lanes.group,
                    surface,
                    source,
                });
            }
        }
    }
    faults
}

/// A case: what it shows, the groups, and the faults.
type Case<'a> = (&'a str, &'a [Lanes<'a>], &'a [LaneFault<'a>]);

#[test]
fn the_lane_source_guard_refuses_each_broken_rule() {
    let reserved = [10..=20, 30..=39];
    let cases: &[Case<'_>] = &[
        (
            "two groups in ranges of their own",
            &[
                Lanes {
                    group: "a",
                    ranges: &[10..=12, 30..=31],
                    sources: &[("x", 10), ("y", 31)],
                },
                Lanes {
                    group: "b",
                    ranges: &[13..=15],
                    sources: &[("z", 15)],
                },
            ],
            &[],
        ),
        (
            "two surfaces of one group under one source",
            &[Lanes {
                group: "a",
                ranges: &[10..=12],
                sources: &[("ports", 11), ("port connections", 11)],
            }],
            &[],
        ),
        (
            "a range that starts below the reservation",
            &[Lanes {
                group: "a",
                ranges: &[9..=12],
                sources: &[],
            }],
            &[LaneFault::OutsideReservation {
                group: "a",
                range: 9..=12,
            }],
        ),
        (
            "a range that ends past the reservation",
            &[Lanes {
                group: "a",
                ranges: &[18..=21],
                sources: &[],
            }],
            &[LaneFault::OutsideReservation {
                group: "a",
                range: 18..=21,
            }],
        ),
        (
            "two groups whose ranges overlap",
            &[
                Lanes {
                    group: "a",
                    ranges: &[10..=12],
                    sources: &[],
                },
                Lanes {
                    group: "b",
                    ranges: &[12..=14],
                    sources: &[],
                },
            ],
            &[LaneFault::Overlap {
                first: "a",
                second: "b",
                tag: 12,
            }],
        ),
        (
            "a source in another group's range",
            &[
                Lanes {
                    group: "a",
                    ranges: &[10..=12],
                    sources: &[("x", 13)],
                },
                Lanes {
                    group: "b",
                    ranges: &[13..=15],
                    sources: &[],
                },
            ],
            &[LaneFault::OutsideGroup {
                group: "a",
                surface: "x",
                source: 13,
            }],
        ),
        (
            "a source outside every range",
            &[Lanes {
                group: "a",
                ranges: &[10..=12],
                sources: &[("x", 25)],
            }],
            &[LaneFault::OutsideGroup {
                group: "a",
                surface: "x",
                source: 25,
            }],
        ),
    ];
    assert_eq!(cases.len(), 7);
    for (what, groups, expected) in cases {
        assert_eq!(lane_faults(groups, &reserved), *expected, "{what}");
    }
}

#[test]
fn each_group_takes_its_lane_sources_from_a_reserved_range_of_its_own() {
    let sources: Vec<Vec<(&str, u8)>> = GROUPS
        .iter()
        .map(|group| {
            group
                .hashed_surfaces
                .iter()
                .filter_map(|surface| surface.source.map(|source| (surface.name, source)))
                .collect()
        })
        .collect();
    let groups: Vec<Lanes<'_>> = GROUPS
        .iter()
        .zip(&sources)
        .map(|(group, sources)| Lanes {
            group: group.name,
            ranges: group.lanes,
            sources,
        })
        .collect();
    assert_eq!(groups.len(), GROUPS.len());
    assert!(
        sources.iter().any(|list| !list.is_empty()),
        "no group declares a lane source"
    );
    assert_eq!(lane_faults(&groups, &HYPERVISOR), []);
}
