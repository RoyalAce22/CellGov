use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::CONFIGURE_IRQ_STATE_BITMAP;
use cellgov_time::GuestTicks;

/// The bitmap of the 4.93 kernel's thread 0: +0x1C0 of its record, which
/// the capture `lv2_logical_ppe_record` places at 0x8000000000368f80.
const THREAD_0_BITMAP: u64 = 0x0036_9140;

/// The bitmap of thread 1: +0x1C0 of the record at 0x8000000000369200.
const THREAD_1_BITMAP: u64 = 0x0036_93c0;

/// The 4.93 GameOS partition's real mode size, `bi.pu.1.rm_size`.
const RM_SIZE: u64 = 0x0800_0000;

/// Real storage at address 0 that holds both kernel bitmaps.
fn storage() -> GuestMemory {
    GuestMemory::new(0x0037_0000)
}

/// The 4.93 kernel's call: `sc 1` [OFW-4.93 lv2_kernel a:0x8000000000007d78], with the
/// logical PPE id 1 in r3, and r4 and r5 for hardware thread `thread`.
fn request(thread: u8) -> HypercallRequest {
    let bitmap = [THREAD_0_BITMAP, THREAD_1_BITMAP][usize::from(thread)];
    let mut inputs = [0; 8];
    inputs[..3].copy_from_slice(&[1, u64::from(thread), bitmap]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: CONFIGURE_IRQ_STATE_BITMAP,
        inputs,
        pc: 0x8000_0000_0000_7d78,
        lr: 0,
        thread: Some(thread),
        problem_state: false,
        tick: GuestTicks::new(52_117),
    }
}

fn with(r: &HypercallRequest, reg: usize, value: u64) -> HypercallRequest {
    let mut changed = *r;
    changed.inputs[reg - 3] = value;
    changed
}

fn configure(r: &HypercallRequest, bitmaps: &IrqStateBitmapState) -> HypercallDisposition {
    configure_bitmap(r, Some("4.93"), bitmaps, &storage())
}

fn fresh(r: &HypercallRequest) -> HypercallDisposition {
    configure(r, &IrqStateBitmapState::default())
}

fn lv1_r3(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, vec![0; 7]))
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: CONFIGURE_IRQ_STATE_BITMAP,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

/// lv1 returns r3 = 0 [OFW-4.93 lv1 a:0x2EA598] and writes no return
/// slot, and copies the
/// thread's 32-byte status shadow, 0, to the bitmap [OFW-4.93 lv1 a:0x2C413C].
fn registered(r: &HypercallRequest, thread: u8, bitmap: u64) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, vec![0; 7]);
    answer.effects.push(Effect::SharedWriteIntent {
        range: ByteRange::new(GuestAddr::new(bitmap), 32).expect("the status doublewords"),
        bytes: WritePayload::new(vec![0; 32]),
        source: r.unit,
        source_time: r.tick,
    });
    answer.interrupt = Some(Box::new(InterruptChange::BitmapRegistered(
        IrqStateBitmapRegistration {
            key: IrqStateBitmapKey { ppe_id: 1, thread },
            lpar_address: bitmap,
        },
    )));
    HypercallDisposition::Answered(answer)
}

#[test]
fn the_4_93_kernel_call_registers_thread_0_and_zeroes_its_status_doublewords() {
    let r = request(0);
    assert_eq!(fresh(&r), registered(&r, 0, THREAD_0_BITMAP));
}

#[test]
fn the_second_thread_registers_its_bitmap_under_r4_1() {
    let r = request(1);
    let bitmaps = {
        let mut held = IrqStateBitmapState::default();
        held.register(IrqStateBitmapRegistration {
            key: IrqStateBitmapKey {
                ppe_id: 1,
                thread: 0,
            },
            lpar_address: THREAD_0_BITMAP,
        });
        held
    };
    assert_eq!(configure(&r, &bitmaps), registered(&r, 1, THREAD_1_BITMAP));
}

#[test]
fn r4_names_the_thread_whichever_thread_makes_the_call() {
    let mut r = request(1);
    r.thread = Some(0);
    assert_eq!(fresh(&r), registered(&r, 1, THREAD_1_BITMAP));
    r.thread = None;
    assert_eq!(fresh(&r), registered(&r, 1, THREAD_1_BITMAP));
}

#[test]
fn values_in_r6_to_r10_do_not_change_the_answer() {
    let mut r = request(0);
    r.inputs[3..].copy_from_slice(&[u64::MAX; 5]);
    assert_eq!(fresh(&r), registered(&r, 0, THREAD_0_BITMAP));
}

/// The object table lookup at [OFW-4.93 lv1 a:0x2EA440-0x2EA628].
#[test]
fn an_r3_other_than_the_logical_ppe_id_gets_lv1s_r3_marked_no_entry() {
    for ppe_id in [0, 2, 3, u64::MAX] {
        let r = with(&request(0), 3, ppe_id);
        assert_eq!(fresh(&r), lv1_r3(0x0000_0001_FFFF_FFFA), "{ppe_id}");
    }
}

/// The compare with 2 at [OFW-4.93 lv1 a:0x2EA634].
#[test]
fn an_r4_not_below_2_gets_lv1s_r4_marked_no_entry() {
    for thread in [2, 255, 256, 1 << 32, u64::MAX] {
        let r = with(&request(0), 4, thread);
        assert_eq!(fresh(&r), lv1_r3(0x0000_0002_FFFF_FFFA), "{thread}");
    }
}

/// The +128 check at [OFW-4.93 lv1 a:0x2EA98C].
#[test]
fn an_r5_of_0_with_no_bitmap_held_gets_lv1s_wrong_state() {
    let r = with(&request(0), 5, 0);
    assert_eq!(fresh(&r), lv1_r3(0x0000_0000_FFFF_FFF5));
}

#[test]
fn an_r5_of_0_after_a_registration_of_either_thread_is_refused() {
    let mut bitmaps = IrqStateBitmapState::default();
    bitmaps.register(IrqStateBitmapRegistration {
        key: IrqStateBitmapKey {
            ppe_id: 1,
            thread: 1,
        },
        lpar_address: THREAD_1_BITMAP,
    });
    let r = with(&request(0), 5, 0);
    assert_eq!(
        configure(&r, &bitmaps),
        refused(&r, "the release of a registered interrupt state bitmap")
    );
}

/// The low 3 bits check at [OFW-4.93 lv1 a:0x2EA648].
#[test]
fn an_r5_that_is_not_8_byte_aligned_gets_lv1s_r5_marked_illegal_parameter() {
    for offset in [1, 2, 4, 7] {
        let r = with(&request(0), 5, THREAD_0_BITMAP + offset);
        assert_eq!(fresh(&r), lv1_r3(0x0000_0004_FFFF_FFEF), "{offset}");
    }
}

#[test]
fn an_8_byte_aligned_r5_off_a_64_byte_boundary_registers() {
    let r = with(&request(0), 5, THREAD_0_BITMAP + 8);
    assert_eq!(fresh(&r), registered(&r, 0, THREAD_0_BITMAP + 8));
}

/// The region lookup returns no region at [OFW-4.93 lv1 a:0x2CE7A4], and the handler
/// answers at [OFW-4.93 lv1 a:0x2EA964].
#[test]
fn an_r5_at_or_past_the_real_mode_size_gets_lv1s_r5_marked_no_entry() {
    for address in [RM_SIZE, RM_SIZE + 0x40, (1 << 42) - 8] {
        let r = with(&request(0), 5, address);
        assert_eq!(fresh(&r), lv1_r3(0x0000_0004_FFFF_FFFA), "{address:#x}");
    }
}

/// The region end check at [OFW-4.93 lv1 a:0x2EA6BC].
#[test]
fn a_bitmap_that_passes_the_real_mode_region_end_gets_lv1s_r5_marked_type_mismatch() {
    for address in [RM_SIZE - 8, RM_SIZE - 56] {
        let r = with(&request(0), 5, address);
        assert_eq!(fresh(&r), lv1_r3(0x0000_0004_FFFF_FFF8), "{address:#x}");
    }
}

#[test]
fn a_bitmap_that_ends_at_the_real_mode_region_end_passes_lv1s_checks() {
    let r = with(&request(0), 5, RM_SIZE - 64);
    assert_eq!(
        fresh(&r),
        refused(
            &r,
            "an interrupt state bitmap whose status doublewords CellGov holds no storage for"
        )
    );
}

#[test]
fn a_bitmap_outside_the_real_mode_region_is_refused() {
    for address in [0x5000_0020_0000, 0x0001_0000_0036_9140] {
        let r = with(&request(0), 5, address);
        assert_eq!(
            fresh(&r),
            refused(
                &r,
                "an interrupt state bitmap outside the partition's real mode region"
            ),
            "{address:#x}"
        );
    }
}

#[test]
fn the_first_failing_check_in_lv1s_order_is_the_answer() {
    let r = request(0);
    let ppe_and_thread = with(&with(&r, 3, 2), 4, 2);
    assert_eq!(fresh(&ppe_and_thread), lv1_r3(0x0000_0001_FFFF_FFFA));
    let thread_and_zero = with(&with(&r, 4, 2), 5, 0);
    assert_eq!(fresh(&thread_and_zero), lv1_r3(0x0000_0002_FFFF_FFFA));
    let misaligned_past_end = with(&r, 5, RM_SIZE + 1);
    assert_eq!(fresh(&misaligned_past_end), lv1_r3(0x0000_0004_FFFF_FFEF));
    let misaligned_outside_real_mode = with(&r, 5, 0x5000_0020_0004);
    assert_eq!(
        fresh(&misaligned_outside_real_mode),
        lv1_r3(0x0000_0004_FFFF_FFEF)
    );
}

#[test]
fn the_r3_and_r4_checks_come_before_the_release_refusal() {
    let mut bitmaps = IrqStateBitmapState::default();
    bitmaps.register(IrqStateBitmapRegistration {
        key: IrqStateBitmapKey {
            ppe_id: 1,
            thread: 0,
        },
        lpar_address: THREAD_0_BITMAP,
    });
    let release = with(&request(0), 5, 0);
    let wrong_ppe = with(&release, 3, 2);
    assert_eq!(
        configure(&wrong_ppe, &bitmaps),
        lv1_r3(0x0000_0001_FFFF_FFFA)
    );
    let wrong_thread = with(&release, 4, 2);
    assert_eq!(
        configure(&wrong_thread, &bitmaps),
        lv1_r3(0x0000_0002_FFFF_FFFA)
    );
}

#[test]
fn a_call_at_a_firmware_whose_lv1_handler_is_not_read_is_refused() {
    let r = request(0);
    for firmware in [Some("3.70"), Some("4.92"), None] {
        assert_eq!(
            configure_bitmap(&r, firmware, &IrqStateBitmapState::default(), &storage()),
            refused(
                &r,
                "an interrupt state bitmap call at a firmware whose lv1 handler is not read"
            ),
            "{firmware:?}"
        );
    }
}

#[test]
fn the_hypervisor_keeps_each_threads_bitmap_and_hashes_it() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let empty = hv.sync_term();
    let r0 = request(0);
    assert_eq!(
        hv.dispatch_with(&r0, &storage()),
        registered(&r0, 0, THREAD_0_BITMAP)
    );
    let one = hv.sync_term();
    let r1 = request(1);
    assert_eq!(
        hv.dispatch_with(&r1, &storage()),
        registered(&r1, 1, THREAD_1_BITMAP)
    );
    let key = |thread| IrqStateBitmapKey { ppe_id: 1, thread };
    assert_eq!(hv.irq_state_bitmaps().bitmaps().len(), 2);
    assert_eq!(hv.irq_state_bitmaps().bitmap(key(0)), Some(THREAD_0_BITMAP));
    assert_eq!(hv.irq_state_bitmaps().bitmap(key(1)), Some(THREAD_1_BITMAP));
    assert_ne!(one, empty);
    assert_ne!(hv.sync_term(), one);
    assert!(hv.witnesses().is_empty());
    assert_eq!(hv.refusal(), None);
}

#[test]
fn a_call_that_fails_a_check_keeps_no_bitmap() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let before = hv.clone();
    let r = with(&request(0), 5, THREAD_0_BITMAP + 4);
    assert_eq!(
        hv.dispatch_with(&r, &storage()),
        lv1_r3(0x0000_0004_FFFF_FFEF)
    );
    assert_eq!(hv, before);
}
