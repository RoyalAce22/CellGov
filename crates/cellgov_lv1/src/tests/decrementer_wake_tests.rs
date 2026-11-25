use super::*;
use crate::group::interrupt::bitmap::{
    IrqStateBitmapKey, IrqStateBitmapRegistration, StatusShadowWrite,
};
use crate::group::interrupt::pause::PausedThread;
use crate::group::interrupt::InterruptChange;
use crate::return_path::{ExternalDelivery, ExternalRaise};
use crate::{HypercallDisposition, HypercallRequest, Hypervisor};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::lv1::call::{
    CONFIGURE_IRQ_STATE_BITMAP, CONNECT_IRQ_PLUG_EXT, CONSTRUCT_EVENT_RECEIVE_PORT,
    SEND_EVENT_LOCALLY,
};
use cellgov_ps3_abi::lv1::interrupt::IRQ_MASK_OFFSET;
use cellgov_ps3_abi::lv1::pause::{PAUSE_MODE_DECREMENTER_IGNORED, PAUSE_MODE_DECREMENTER_WAKES};
use cellgov_ps3_abi::lv1::ppe::game_os_logical_ppe_id;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's idle thread
/// [OFW-4.93 lv2_kernel a:0x8000000000265184].
const IDLE_SC: u64 = 0x8000_0000_0026_5184;

const UNIT: UnitId = UnitId::new(2);

const THREAD_1_BITMAP: u64 = 0x0036_9200;

/// Plug 9 is bit 63 - 9 of status doubleword 0 (`plug_status_bit`).
const PLUG_BIT: u64 = 1 << 54;

/// The 4.93 kernel's MSR at its idle `pause`: SF, EE, IR, DR and RI.
const KERNEL_MSR: u64 = 0x8000_0000_0000_8032;

/// lv1's raise clears bits 48 to 63 of [`KERNEL_MSR`] and keeps SF
/// [OFW-4.93 lv1 a:0x18FC-0x1914], and enters 0x500
/// [OFW-4.93 lv1 a:0x2C3DB8].
const DELIVERY: ExternalRaise = ExternalRaise::Deliver(ExternalDelivery {
    vector: 0x500,
    msr: 0x8000_0000_0000_0000,
});

fn wake(thread: u8) -> DecrementerWake {
    DecrementerWake {
        unit: UNIT,
        thread,
        pc: IDLE_SC,
    }
}

fn pause(thread: u8, mode: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[0] = mode;
    HypercallRequest {
        unit: UNIT,
        number: PAUSE,
        inputs,
        pc: IDLE_SC,
        lr: 0,
        thread: Some(thread),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

/// A 4.93 hypervisor in which thread 1 paused in `mode`.
fn paused_in(mode: u64) -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let disposition = hv.dispatch(&pause(1, mode));
    assert!(
        matches!(&disposition, HypercallDisposition::Answered(a) if a.pauses_caller),
        "{disposition:?}"
    );
    hv
}

/// Storage that holds `mask` in mask doubleword 0 of thread 1's bitmap.
fn storage(mask: u64) -> GuestMemory {
    let mut storage = GuestMemory::new(0x0037_0000);
    let address = THREAD_1_BITMAP + IRQ_MASK_OFFSET;
    let range = ByteRange::new(GuestAddr::new(address), 8).expect("the mask doubleword");
    storage
        .apply_commit(range, &mask.to_be_bytes())
        .expect("the mask write");
    storage
}

/// 4.93 group states where thread 1 registered its bitmap, holds
/// [`PLUG_BIT`] in its status shadow, and paused in mode 0.
fn raised_while_paused() -> GroupStates {
    let key = IrqStateBitmapKey {
        ppe_id: game_os_logical_ppe_id("4.93").expect("the 4.93 logical PPE id"),
        thread: 1,
    };
    let mut groups = GroupStates::default();
    for change in [
        InterruptChange::BitmapRegistered(IrqStateBitmapRegistration {
            key,
            lpar_address: THREAD_1_BITMAP,
        }),
        InterruptChange::StatusShadow(StatusShadowWrite {
            key,
            index: 0,
            value: PLUG_BIT,
        }),
        InterruptChange::ThreadPaused(PausedThread {
            thread: 1,
            decrementer_wakes: true,
        }),
    ] {
        groups.apply(change.into());
    }
    groups
}

#[test]
fn a_decrementer_wake_ends_a_mode_0_pause_and_resumes_the_thread() {
    let mut hv = paused_in(PAUSE_MODE_DECREMENTER_WAKES);
    let resume = hv.wake_on_decrementer(&wake(1), &GuestMemory::new(0), &SavedMsrs::default());
    assert_eq!(
        resume,
        Ok(Some(ThreadResume {
            thread: 1,
            external: None,
            clears_decrementer: true,
        }))
    );
    assert_eq!(hv.groups().interrupt().paused().thread(1), None);
    assert_eq!(hv.refusal(), None);
}

#[test]
fn a_decrementer_wake_leaves_a_mode_1_pause_and_an_unpaused_thread_as_they_are() {
    let mut hv = paused_in(PAUSE_MODE_DECREMENTER_IGNORED);
    let before = hv.groups().clone();
    for thread in [1, 0] {
        let resume =
            hv.wake_on_decrementer(&wake(thread), &GuestMemory::new(0), &SavedMsrs::default());
        assert_eq!(resume, Ok(None));
    }
    assert_eq!(hv.groups(), &before);
}

/// The woken thread returns through the vector of its call, which raises
/// the external interrupt for a shadow bit that the mask holds
/// [OFW-4.93 lv1 a:0x0FC4-0x101C].
#[test]
fn a_decrementer_wake_raises_the_external_interrupt_of_an_unmasked_shadow_bit() {
    let msrs = SavedMsrs([None, Some(KERNEL_MSR)]);
    let resume = wake_entry(
        &wake(1),
        Some("4.93"),
        &raised_while_paused(),
        &storage(PLUG_BIT),
        &msrs,
    );
    assert_eq!(
        resume,
        Ok(Some(ThreadResume {
            thread: 1,
            external: Some(DELIVERY),
            clears_decrementer: true,
        }))
    );
}

#[test]
fn a_decrementer_wake_whose_return_has_no_saved_msr_refuses_the_pause() {
    let resume = wake_entry(
        &wake(1),
        Some("4.93"),
        &raised_while_paused(),
        &storage(PLUG_BIT),
        &SavedMsrs::default(),
    );
    assert_eq!(
        resume,
        Err(HypercallRefusal::UnmodelledInput {
            number: PAUSE,
            unit: UNIT,
            pc: IDLE_SC,
            what: "an external interrupt on the return to a thread whose unit gives no saved MSR",
        })
    );
}

/// The `sc` of the kernel's port call in the setup of a thread's record
/// [OFW-4.93 lv2_kernel a:0x8000000000007d8c]. The console gave thread 1's
/// call the port id 4 (`GAME_OS_PORT_CALLS`).
const RECORD_SC: u64 = 0x8000_0000_0000_7d8c;

/// The port of thread 1's record setup.
const PORT: u64 = 4;

/// The plug of [`PLUG_BIT`].
const PLUG: u64 = 9;

fn call(number: u64, args: &[u64], thread: u8, pc: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..args.len()].copy_from_slice(args);
    HypercallRequest {
        number,
        inputs,
        pc,
        thread: Some(thread),
        ..pause(thread, PAUSE_MODE_DECREMENTER_WAKES)
    }
}

/// A 4.93 hypervisor where thread 1 registered its bitmap and paused in
/// mode 0, and a raise of [`PLUG`] then set [`PLUG_BIT`] in its status
/// shadow while a mask of 0 kept the thread paused.
fn masked_raise_while_paused() -> Hypervisor {
    let ppe_id = game_os_logical_ppe_id("4.93").expect("the 4.93 logical PPE id");
    let masked = storage(0);
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    for request in [
        call(
            CONFIGURE_IRQ_STATE_BITMAP,
            &[ppe_id, 1, THREAD_1_BITMAP],
            1,
            IDLE_SC,
        ),
        call(CONSTRUCT_EVENT_RECEIVE_PORT, &[], 1, RECORD_SC),
        call(
            CONNECT_IRQ_PLUG_EXT,
            &[ppe_id, 1, PLUG, PORT, 0],
            0,
            IDLE_SC,
        ),
        pause(1, PAUSE_MODE_DECREMENTER_WAKES),
        call(SEND_EVENT_LOCALLY, &[PORT], 0, IDLE_SC),
    ] {
        let disposition = hv.dispatch_with(&request, &masked);
        assert!(
            matches!(&disposition, HypercallDisposition::Answered(a) if a.r3 == 0),
            "{disposition:?}"
        );
    }
    let key = IrqStateBitmapKey { ppe_id, thread: 1 };
    let interrupt = hv.groups().interrupt();
    assert_eq!(interrupt.bitmaps().shadow(key), [PLUG_BIT, 0, 0, 0]);
    assert_eq!(
        interrupt.paused().thread(1),
        Some(PausedThread {
            thread: 1,
            decrementer_wakes: true,
        })
    );
    hv
}

#[test]
fn a_refused_decrementer_wake_keeps_the_refusal_and_the_pause() {
    let mut hv = masked_raise_while_paused();
    let before = hv.groups().clone();
    let refusal = HypercallRefusal::UnmodelledInput {
        number: PAUSE,
        unit: UNIT,
        pc: IDLE_SC,
        what: "an external interrupt on the return to a thread whose unit gives no saved MSR",
    };
    let resume = hv.wake_on_decrementer(&wake(1), &storage(PLUG_BIT), &SavedMsrs::default());
    assert_eq!(resume, Err(refusal));
    assert_eq!(hv.refusal(), Some(refusal));
    assert_eq!(hv.groups(), &before);
}

#[test]
fn a_decrementer_wake_after_a_masked_raise_delivers_the_bit_that_the_mask_now_holds() {
    let mut hv = masked_raise_while_paused();
    let msrs = SavedMsrs([None, Some(KERNEL_MSR)]);
    let resume = hv.wake_on_decrementer(&wake(1), &storage(PLUG_BIT), &msrs);
    assert_eq!(
        resume,
        Ok(Some(ThreadResume {
            thread: 1,
            external: Some(DELIVERY),
            clears_decrementer: true,
        }))
    );
    assert_eq!(hv.groups().interrupt().paused().thread(1), None);
    assert_eq!(hv.refusal(), None);
}
