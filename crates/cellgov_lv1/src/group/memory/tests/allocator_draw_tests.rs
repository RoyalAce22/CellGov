use super::*;

/// 1 MB: e = 20, so the address holds 2^22 counter values.
const MB: u64 = 0x10_0000;

fn start() -> u64 {
    game_os_region_counter_at_boot("4.93").expect("the 4.93 start value")
}

fn draw(memory: &MemoryState) -> CounterDraw {
    CounterDraw::of(memory, "4.93").expect("a 4.93 draw")
}

fn counter(draw: &mut CounterDraw, size: u64) -> u64 {
    draw.place(size, &HeldRegions::default())
        .expect("a placement")
        .counter
}

#[test]
fn a_draw_with_no_taken_value_starts_at_the_partitions_start_value() {
    let mut draw = draw(&MemoryState::default());
    assert_eq!(
        draw.place(MB, &HeldRegions::default()),
        Ok(Placement {
            address: region_address(MB, start()).expect("an address"),
            counter: start(),
        })
    );
}

#[test]
fn a_draw_after_a_taken_value_starts_one_past_it() {
    let mut memory = MemoryState::default();
    memory.take_counter(start() + 7);
    assert_eq!(counter(&mut draw(&memory), MB), start() + 8);
}

#[test]
fn consecutive_placements_from_one_draw_take_consecutive_values() {
    let mut draw = draw(&MemoryState::default());
    let first = counter(&mut draw, MB);
    let second = counter(&mut draw, 0x1000);
    assert_eq!((first, second), (start(), start() + 1));
}

#[test]
fn a_refused_placement_leaves_the_draw_as_it_was() {
    let mut draw = draw(&MemoryState::default());
    assert_eq!(
        draw.place((1 << 42) + 1, &HeldRegions::default()),
        Err(NoPlacement::TooLarge)
    );
    assert_eq!(counter(&mut draw, MB), start());
}

#[test]
fn a_draw_at_a_firmware_with_no_start_value_is_none() {
    assert_eq!(game_os_region_counter_at_boot("3.41"), None);
    assert_eq!(CounterDraw::of(&MemoryState::default(), "3.41"), None);
}
