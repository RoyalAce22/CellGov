use super::*;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::interrupt::IRQ_STATE_BITMAP_HANDLERS;
use cellgov_time::GuestTicks;

/// The bitmap of the 4.93 kernel's thread 0: +0x1C0 of its record, which
/// the capture `lv2_logical_ppe_record` places at 0x8000000000368f80.
const THREAD_0_BITMAP: u64 = 0x0036_9140;

/// The 4.93 kernel's call for thread 0: `sc 1` [OFW-4.93 lv2_kernel a:0x8000000000007d78].
fn request() -> HypercallRequest {
    let ppe_id = game_os_logical_ppe_id("4.93").expect("the 4.93 logical PPE id");
    let mut inputs = [0; 8];
    inputs[..3].copy_from_slice(&[ppe_id, 0, THREAD_0_BITMAP]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: CONFIGURE_IRQ_STATE_BITMAP,
        inputs,
        pc: 0x8000_0000_0000_7d78,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(52_117),
    }
}

/// While this holds, the bitmap handler does not reach its refusal of a
/// firmware whose real mode size no source gives.
#[test]
fn every_firmware_with_a_bitmap_handler_row_has_a_real_mode_size() {
    let firmwares: Vec<&str> = IRQ_STATE_BITMAP_HANDLERS
        .iter()
        .map(|row| row.firmware)
        .collect();
    assert!(!firmwares.is_empty(), "no bitmap handler row");
    let unsourced: Vec<&str> = firmwares
        .into_iter()
        .filter(|fw| memory::real_mode_size(fw).is_none())
        .collect();
    assert_eq!(unsourced, Vec::<&str>::new());
}

#[test]
fn a_handler_row_firmware_with_no_real_mode_size_source_is_refused() {
    let r = request();
    let storage = GuestMemory::new(0x0037_0000);
    let bitmaps = IrqStateBitmapState::default();
    let HypercallDisposition::Answered(answer) =
        configure_bitmap_with(&r, Some("4.93"), &bitmaps, &storage, memory::real_mode_size)
    else {
        panic!("the 4.93 call with the real mode size of the 4.93 tree is a refusal");
    };
    assert_eq!(answer.r3, 0);
    assert_eq!(
        configure_bitmap_with(&r, Some("4.93"), &bitmaps, &storage, |_: &str| None),
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
            number: CONFIGURE_IRQ_STATE_BITMAP,
            unit: r.unit,
            pc: r.pc,
            what: "an interrupt state bitmap at a firmware whose real mode size no source gives",
        })
    );
}
