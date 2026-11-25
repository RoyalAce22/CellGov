use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use crate::timing::{classify, PpeTiming};
use cellgov_ps3_abi::hw::ppu_issue as k;
use cellgov_ps3_abi::lv2::memory::page_size;
use cellgov_ps3_abi::lv2::syscall as sc;

/// The cycles from one `sc` of `number` with `r3` and `r4`, returning
/// `code`, to the issue of an `addi` behind it.
fn trip(number: u64, r3: u64, r4: u64, code: u64) -> u64 {
    resumed_trip(number, r3, r4, code, None)
}

/// [`trip`] for a thread that left the PPE during the call, when
/// `resumed` is `Some`. `Some(true)` means it left only for a
/// higher-priority thread.
fn resumed_trip(number: u64, r3: u64, r4: u64, code: u64, resumed: Option<bool>) -> u64 {
    let mut state = PpuState::new();
    state.set_gpr(11, number);
    state.set_gpr(3, r3);
    state.set_gpr(4, r4);
    let mut timing = PpeTiming::new();
    let (ops, n) = classify(&I::Sc { lev: 0 }, &state);
    let issued = timing.retire_instruction(0x1000, &ops[..n], 0x1004)[0];
    timing.note_syscall_return(code);
    if let Some(preempted) = resumed {
        timing.note_syscall_resumed(preempted);
    }
    timing.charge_syscall();
    let addi = I::Addi {
        rt: 17,
        ra: 17,
        imm: 1,
    };
    let (ops, n) = classify(&addi, &state);
    timing.retire_instruction(0x1004, &ops[..n], 0x1008)[0] - issued
}

#[test]
fn an_allocation_takes_the_round_trip_of_its_size_and_page_size() {
    // The cycles of the call's thread beside its round trip.
    let rest = trip(sc::TIME_GET_TIMEBASE_FREQUENCY, 0, 0, 0)
        - k::ppu_syscall_round_trip_cycles(sc::TIME_GET_TIMEBASE_FREQUENCY);
    for (size, flags, (base, per_mib)) in [
        (
            1 << 20,
            page_size::FLAG_1M,
            k::PPU_MEMORY_ALLOCATE_1M_PAGE_CYCLES,
        ),
        (
            16 << 20,
            page_size::FLAG_1M,
            k::PPU_MEMORY_ALLOCATE_1M_PAGE_CYCLES,
        ),
        (16 << 20, 0, k::PPU_MEMORY_ALLOCATE_1M_PAGE_CYCLES),
        (
            64 << 10,
            page_size::FLAG_64K,
            k::PPU_MEMORY_ALLOCATE_64K_PAGE_CYCLES,
        ),
        (
            4 << 20,
            page_size::FLAG_64K,
            k::PPU_MEMORY_ALLOCATE_64K_PAGE_CYCLES,
        ),
    ] {
        assert_eq!(
            trip(sc::MEMORY_ALLOCATE, size, flags, 0),
            rest + base + per_mib * size / (1 << 20),
            "{size:#x} bytes under flags {flags:#x}",
        );
    }
}

#[test]
fn a_refused_allocation_keeps_its_error_trip() {
    // A refusal no capture times keeps the call's table entry.
    let rest = trip(sc::TIME_GET_TIMEBASE_FREQUENCY, 0, 0, 0)
        - k::ppu_syscall_round_trip_cycles(sc::TIME_GET_TIMEBASE_FREQUENCY);
    let einval = u64::from(cellgov_ps3_abi::lv2::errno::CELL_EINVAL);
    assert_eq!(
        trip(sc::MEMORY_ALLOCATE, 16 << 20, page_size::FLAG_1M, einval),
        rest + k::ppu_syscall_round_trip_cycles(sc::MEMORY_ALLOCATE),
        "pins the flat table entry: the console takes far less to refuse an allocation",
    );
}

#[test]
fn a_preempted_allocation_takes_its_size_s_trip_and_the_resume_wait() {
    for (size, flags) in [
        (1 << 20, page_size::FLAG_1M),
        (16 << 20, page_size::FLAG_1M),
        (4 << 20, page_size::FLAG_64K),
    ] {
        assert_eq!(
            resumed_trip(sc::MEMORY_ALLOCATE, size, flags, 0, Some(true)),
            trip(sc::MEMORY_ALLOCATE, size, flags, 0) + k::PPU_PREEMPTED_RESUME_CYCLES,
            "{size:#x} bytes under flags {flags:#x}",
        );
    }
}

#[test]
fn a_preempted_refused_allocation_takes_its_error_trip_and_the_resume_wait() {
    let einval = u64::from(cellgov_ps3_abi::lv2::errno::CELL_EINVAL);
    let refused = trip(sc::MEMORY_ALLOCATE, 16 << 20, page_size::FLAG_1M, einval);
    assert_ne!(
        refused,
        trip(sc::MEMORY_ALLOCATE, 16 << 20, page_size::FLAG_1M, 0),
        "a refusal and a success of this size take different trips",
    );
    assert_eq!(
        resumed_trip(
            sc::MEMORY_ALLOCATE,
            16 << 20,
            page_size::FLAG_1M,
            einval,
            Some(true),
        ),
        refused + k::PPU_PREEMPTED_RESUME_CYCLES,
    );
}

#[test]
fn a_call_other_than_an_allocation_ignores_its_arguments() {
    for (r3, r4) in [(0, 0), (16 << 20, page_size::FLAG_64K)] {
        assert_eq!(
            trip(sc::MEMORY_FREE, r3, r4, 0),
            trip(sc::MEMORY_FREE, 0, 0, 0),
        );
    }
}
