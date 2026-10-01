use super::*;
use crate::observation::NamedMemoryRegion;
use crate::test_support::sample_observation;

fn range(region: &str, offset: u64, size: u64) -> VolatileRange {
    VolatileRange {
        region: region.to_string(),
        offset,
        size,
        reason: "varies".to_string(),
    }
}

fn observation_with(regions: &[(&str, Vec<u8>)]) -> Observation {
    let mut observation = sample_observation();
    observation.memory_regions = regions
        .iter()
        .map(|(name, data)| NamedMemoryRegion {
            name: (*name).to_string(),
            addr: 0,
            data: data.clone(),
        })
        .collect();
    observation
}

fn data(observation: &Observation) -> Vec<Vec<u8>> {
    observation
        .memory_regions
        .iter()
        .map(|r| r.data.clone())
        .collect()
}

#[test]
fn only_the_declared_bytes_of_the_named_region_are_zeroed() {
    let mut observation = observation_with(&[("result", vec![1; 8]), ("other", vec![2; 4])]);
    blank_volatile(
        &mut observation,
        &[range("result", 2, 3), range("result", 7, 1)],
    );
    assert_eq!(
        data(&observation),
        [vec![1, 1, 0, 0, 0, 1, 1, 0], vec![2, 2, 2, 2]]
    );
}

#[test]
fn a_range_past_the_data_or_naming_an_absent_region_blanks_only_what_overlaps() {
    let mut observation = observation_with(&[("result", vec![1; 4])]);
    blank_volatile(
        &mut observation,
        &[
            range("result", 3, 10),
            range("result", 9, 2),
            range("result", u64::MAX, 2),
            range("missing", 0, 4),
        ],
    );
    assert_eq!(data(&observation), [vec![1, 1, 1, 0]]);
}

#[test]
fn two_observations_differing_only_in_volatile_bytes_compare_equal_after_blanking() {
    let ranges = [range("result", 4, 4)];
    let mut console = observation_with(&[("result", vec![0, 0, 0, 1, 0xAA, 0xBB, 0xCC, 0xDD])]);
    let mut emulator = observation_with(&[("result", vec![0, 0, 0, 1, 0, 0, 0, 7])]);
    assert_ne!(console.memory_regions, emulator.memory_regions);
    blank_volatile(&mut console, &ranges);
    blank_volatile(&mut emulator, &ranges);
    assert_eq!(console.memory_regions, emulator.memory_regions);
}
