//! PPU stores into Raw SPU windows, checked against the
//! `spu_raw_window_span` console capture.
//!
//! On the console:
//!
//! 1. An aligned `stw` into a Raw SPU's local store completes.
//! 2. A `std` four bytes past a doubleword in the local store ends the
//!    child thread at the store.
//! 3. A `std` that runs from Raw SPU 0's window into Raw SPU 1's ends the
//!    child thread at the store.
//!
//! Cases 2 and 3 change no local-store byte. The windows are
//! caching-inhibited, so CellGov takes the alignment interrupt at both.
//! Each case is its own run here; the console ran each in its own thread.

use cellgov_effects::FaultKind;
use cellgov_mem::{ByteRange, GuestAddr, PageSize, Region, RegionAccess};
use cellgov_ppu::PpuFaultClass;
use cellgov_ps3_abi::lv2::errno::CELL_ESRCH;
use cellgov_ps3_abi::lv2::spu::raw_window;
use cellgov_ps3_abi::lv2::syscall as sc;
use cellgov_spu::SpuExecutionUnit;

use crate::span_harness::{call, doubleword, exit, li, oris, payload, run, std, stw, word, OUT};

/// The done marker, "RAWS".
const DONE: u32 = 0x5241_5753;
/// The value the probe writes for a join it did not make.
const SKIP: u32 = 0xDEAD_D00D;
/// The value the probe's child passes to its thread exit.
const CHILD_EXIT: u32 = 0xE5;

/// The probe's fill of Raw SPU 1's local store words 0..3.
const FILL: [u32; 4] = [0xA0A1_A2A3, 0xB0B1_B2B3, 0xC0C1_C2C3, 0xD0D1_D2D3];

/// A case's words in the payload.
struct Case {
    passed: bool,
    ls: [u32; 4],
}

/// Reads case `n` (1-based) and asserts the words its outcome fixes.
fn case(w: &[u32], n: usize) -> Case {
    let c = &w[9 + 11 * (n - 1)..][..11];
    assert_eq!(c[0..3], [0, 0, 1], "case {n}: create, start, reached");
    let passed = c[3] == 1;
    if passed {
        assert_eq!(c[4..7], [0, 0, CHILD_EXIT], "case {n}: the child exited");
    } else {
        assert_eq!(c[3], 0, "case {n}: the passed flag");
        assert_eq!(c[4], CELL_ESRCH.code, "case {n}: the child is gone");
        assert_eq!(c[5..7], [SKIP, SKIP], "case {n}: no join");
    }
    Case {
        passed,
        ls: c[7..11].try_into().expect("4"),
    }
}

/// Memory as the boot gives it: main memory and the reserved SPU range,
/// which is caching-inhibited and holds two windows here.
fn regions() -> Vec<Region> {
    vec![
        Region::new(0, 0x2_0000, "main", PageSize::Page64K),
        Region::with_access(
            raw_window::BASE,
            2 * raw_window::STRIDE as usize,
            "spu_reserved",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        )
        .with_caching_inhibited(),
    ]
}

/// The probe's setup, then `access` with r5 at Raw SPU 1's window, then
/// `sys_process_exit`. r14..r16 hold the call return codes.
fn program(access: &[u32]) -> Vec<u32> {
    let mut p = Vec::new();
    call(&mut p, sc::SPU_INITIALIZE, &[li(3, 6), li(4, 2)], 14);
    for (slot, rc) in [(0i16, 15u32), (8, 16)] {
        let args = [li(3, OUT as i16 + slot), li(4, 0)];
        call(&mut p, sc::RAW_SPU_CREATE, &args, rc);
    }
    p.push(li(5, 0));
    p.push(oris(
        5,
        5,
        ((raw_window::BASE + raw_window::STRIDE) >> 16) as u16,
    ));
    for (i, value) in FILL.iter().enumerate() {
        word(&mut p, 6, *value);
        p.push(stw(6, 4 * i as i16, 5));
    }
    p.extend_from_slice(access);
    exit(&mut p);
    p
}

/// Runs case `access` against the capture `w` and returns its first fault
/// class and Raw SPU 1's local store words 0..3.
fn cellgov(w: &[u32], access: &[u32]) -> (Option<PpuFaultClass>, [u32; 4]) {
    let ran = run(regions(), &program(access));
    let rcs: Vec<u64> = [w[2], w[3], w[5]].iter().map(|&v| u64::from(v)).collect();
    assert_eq!(ran.gpr()[14..17], rcs[..], "initialize and the two creates");
    // The program stores into window 1 and reads Raw SPU 1, so the two
    // creates must give the console's ids.
    let id_at = |slot: u64| {
        let range = ByteRange::new(GuestAddr::new(OUT + slot), 4).expect("range");
        let bytes = ran.rt.memory().read(range).expect("the id slot");
        u32::from_be_bytes(bytes.try_into().expect("4"))
    };
    assert_eq!([id_at(0), id_at(8)], [w[4], w[6]], "the Raw SPU ids");
    let raw = ran.rt.lv2_host().raw_spu_unit(1).expect("Raw SPU 1");
    let spu = ran.rt.registry().get(raw).expect("its unit").as_any();
    let ls = &spu
        .downcast_ref::<SpuExecutionUnit>()
        .expect("an SPU unit")
        .state()
        .ls;
    let words =
        std::array::from_fn(|i| u32::from_be_bytes(ls[4 * i..4 * i + 4].try_into().expect("4")));
    let class = ran.first_fault.map(|(unit, kind)| {
        assert_eq!(unit, ran.ppu, "the PPU faults");
        match kind {
            FaultKind::Guest(code) => PpuFaultClass::of_code(code).expect("a PPU fault code"),
            other => panic!("{other:?}"),
        }
    });
    (class, words)
}

#[test]
fn stores_into_and_across_raw_spu_windows_complete_or_take_the_alignment_interrupt_as_on_the_console(
) {
    let w = payload("spu_raw_window_span", 48);
    assert_eq!(w[0..2], [DONE, 0], "the main thread finished");
    assert_eq!(
        w[2..9],
        [0, 0, 0, 0, 1, 0, 0],
        "Raw SPUs 0 and 1 and their destroys"
    );

    // Case 1's store; the console ran it before cases 2 and 3.
    let mut stw_word = Vec::new();
    word(&mut stw_word, 6, 0x5A5A_5A5A);
    stw_word.push(stw(6, 12, 5));
    let after_stw = |ds: i16| {
        let mut p = stw_word.clone();
        doubleword(&mut p, 7);
        p.push(std(7, ds, 5));
        p
    };
    // (case, the accesses, the class CellGov must take)
    let cases = [
        (1, stw_word.clone(), None),
        (2, after_stw(4), Some(PpuFaultClass::Alignment)),
        (3, after_stw(-4), Some(PpuFaultClass::Alignment)),
    ];
    let mut before = FILL;
    for (n, access, class) in cases {
        let console = case(&w, n);
        assert_eq!(console.passed, class.is_none(), "case {n}");
        if !console.passed {
            assert_eq!(console.ls, before, "case {n}: no byte landed");
        }
        assert_eq!(cellgov(&w, &access), (class, console.ls), "case {n}");
        before = console.ls;
    }
}
