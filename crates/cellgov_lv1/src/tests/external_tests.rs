use crate::return_path::{ExternalDelivery, ReturnPair, SavedMsrs};
use crate::{
    HypercallAnswer, HypercallDisposition, HypercallRefusal, HypercallRequest, Hypervisor,
    IrqStateBitmapKey, ThreadResume,
};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv1::call::{
    CONFIGURE_IRQ_STATE_BITMAP, CONNECT_IRQ_PLUG_EXT, CONSTRUCT_EVENT_RECEIVE_PORT, PAUSE,
    SEND_EVENT_LOCALLY,
};
use cellgov_ps3_abi::lv1::interrupt::IRQ_MASK_OFFSET;
use cellgov_time::GuestTicks;

/// The `sc` of the kernel's port call in the setup of a thread's record
/// [OFW-4.93 lv2_kernel a:0x8000000000007d8c]. The console gave thread 1's
/// call the port id 4 (`GAME_OS_PORT_CALLS`).
const RECORD_SC: u64 = 0x8000_0000_0000_7d8c;
const PORT: u64 = 4;

/// The `sc` of the kernel's event send
/// [OFW-4.93 lv2_kernel a:0x8000000000025a34].
const SEND_SC: u64 = 0x8000_0000_0002_5a34;

const THREAD_1_BITMAP: u64 = 0x0036_9200;

/// Plug 9 is bit 63 - 9 of status doubleword 0 (`plug_status_bit`).
const PLUG: u64 = 9;
const PLUG_BIT: u64 = 1 << 54;

/// The 4.93 kernel's MSR at its first dispatcher manager write: SF, EE,
/// IR, DR and RI.
const KERNEL_MSR: u64 = 0x8000_0000_0000_8032;

/// lv1's raise clears bits 48 to 63 of [`KERNEL_MSR`] and keeps SF
/// [OFW-4.93 lv1 a:0x18FC-0x1914], and enters 0x500
/// [OFW-4.93 lv1 a:0x2C3DB8].
const DELIVERY: ExternalDelivery = ExternalDelivery {
    vector: 0x500,
    msr: 0x8000_0000_0000_0000,
};

fn call(number: u64, args: &[u64], thread: u8) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..args.len()].copy_from_slice(args);
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs,
        pc: SEND_SC,
        lr: 0,
        thread: Some(thread),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

/// Storage whose thread 1 bitmap holds `mask` in mask doubleword 0.
fn storage(mask: u64) -> GuestMemory {
    let mut storage = GuestMemory::new(0x0037_0000);
    let address = THREAD_1_BITMAP + IRQ_MASK_OFFSET;
    let range = ByteRange::new(GuestAddr::new(address), 8).expect("the mask doubleword");
    storage
        .apply_commit(range, &mask.to_be_bytes())
        .expect("the mask write");
    storage
}

fn answered(disposition: HypercallDisposition) -> HypercallAnswer {
    match disposition {
        HypercallDisposition::Answered(answer) => answer,
        refused => panic!("{refused:?}"),
    }
}

fn msrs(thread_0: Option<u64>, thread_1: Option<u64>) -> SavedMsrs {
    SavedMsrs([thread_0, thread_1])
}

fn thread_1() -> IrqStateBitmapKey {
    IrqStateBitmapKey {
        ppe_id: 1,
        thread: 1,
    }
}

/// A 4.93 hypervisor after thread 1 registers its bitmap and constructs
/// [`PORT`], and thread 0 connects the port to [`PLUG`] of thread 1.
fn booted(storage: &GuestMemory) -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let bitmap = call(CONFIGURE_IRQ_STATE_BITMAP, &[1, 1, THREAD_1_BITMAP], 1);
    answered(hv.dispatch_with(&bitmap, storage));
    let port = HypercallRequest {
        pc: RECORD_SC,
        ..call(CONSTRUCT_EVENT_RECEIVE_PORT, &[], 1)
    };
    answered(hv.dispatch_with(&port, storage));
    let plug = call(CONNECT_IRQ_PLUG_EXT, &[1, 1, PLUG, PORT, 0], 0);
    answered(hv.dispatch_with(&plug, storage));
    hv
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: r.number,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

/// The return of a call that raises an unmasked bit of its caller enters
/// the vector, with the vector in r11 and the new MSR in r12
/// [OFW-4.93 lv1 a:0x1038-0x103C].
#[test]
fn a_raise_on_the_caller_s_unmasked_plug_delivers_its_external_interrupt() {
    let storage = storage(PLUG_BIT);
    let mut hv = booted(&storage);
    let send = call(SEND_EVENT_LOCALLY, &[PORT], 1);
    let answer = answered(hv.dispatch_returning(&send, &storage, &msrs(None, Some(KERNEL_MSR))));
    assert_eq!(answer.external, Some(DELIVERY));
    assert_eq!(answer.resumed, Vec::<ThreadResume>::new());
    assert_eq!(
        answer.return_registers(&send, Some("4.93")).r11_r12,
        ReturnPair::Values {
            r11: 0x500,
            r12: 0x8000_0000_0000_0000,
        }
    );
}

#[test]
fn a_raise_on_a_masked_plug_of_the_caller_delivers_nothing() {
    let storage = storage(0);
    let mut hv = booted(&storage);
    let send = call(SEND_EVENT_LOCALLY, &[PORT], 1);
    let answer = answered(hv.dispatch_returning(&send, &storage, &msrs(None, Some(KERNEL_MSR))));
    assert_eq!(answer.external, None);
    assert_eq!(
        answer.return_registers(&send, Some("4.93")).r11_r12,
        ReturnPair::SaveRestore
    );
    assert_eq!(hv.irq_state_bitmaps().shadow(thread_1())[0], PLUG_BIT);
}

/// With `MSR[EE]` = 0 lv1 sets LPCR[MER] [OFW-4.93 lv1 a:0x191C-0x1928].
/// The refusal answers nothing, so the shadow keeps its value.
#[test]
fn a_delivery_to_a_caller_with_ee_clear_is_a_refusal_that_changes_nothing() {
    let storage = storage(PLUG_BIT);
    let mut hv = booted(&storage);
    let send = call(SEND_EVENT_LOCALLY, &[PORT], 1);
    let msr = KERNEL_MSR & !0x8000;
    assert_eq!(
        hv.dispatch_returning(&send, &storage, &msrs(None, Some(msr))),
        refused(
            &send,
            "an external interrupt on the return to a thread whose saved MSR[EE] is 0, for which \
             lv1 sets LPCR[MER]"
        )
    );
    assert_eq!(hv.irq_state_bitmaps().shadow(thread_1()), [0; 4]);
}

/// lv1 clears ME, bit 51 [OFW-4.93 lv1 a:0x1904], and the PPU's entry
/// keeps it [CBE-Handbook p:292 s:9.12].
#[test]
fn a_delivery_to_a_caller_with_me_set_is_a_refusal() {
    let storage = storage(PLUG_BIT);
    let mut hv = booted(&storage);
    let send = call(SEND_EVENT_LOCALLY, &[PORT], 1);
    assert_eq!(
        hv.dispatch_returning(&send, &storage, &msrs(None, Some(KERNEL_MSR | 0x1000))),
        refused(
            &send,
            "an external interrupt on the return to a thread whose saved MSR holds a bit of 48 to \
             63 that lv1 clears and the PPU's interrupt entry keeps"
        )
    );
}

#[test]
fn a_delivery_to_a_caller_with_no_saved_msr_is_a_refusal() {
    let storage = storage(PLUG_BIT);
    let mut hv = booted(&storage);
    let send = call(SEND_EVENT_LOCALLY, &[PORT], 1);
    assert_eq!(
        hv.dispatch_with(&send, &storage),
        refused(
            &send,
            "an external interrupt on the return to a thread whose unit gives no saved MSR"
        )
    );
}

/// The raise on a paused thread whose mask holds the bit wakes it
/// [OFW-4.93 lv1 a:0x2C6680-0x2C66DC]. The thread returns from `pause`
/// through lv1's vector, which delivers the interrupt.
#[test]
fn a_raise_that_wakes_a_paused_thread_ends_its_pause_and_delivers_its_interrupt() {
    let storage = storage(PLUG_BIT);
    let mut hv = booted(&storage);
    answered(hv.dispatch_with(&call(PAUSE, &[0], 1), &storage));
    assert!(hv.groups().interrupt().paused().thread(1).is_some());
    let send = call(SEND_EVENT_LOCALLY, &[PORT], 0);
    let saved = msrs(Some(KERNEL_MSR), Some(KERNEL_MSR));
    let answer = answered(hv.dispatch_returning(&send, &storage, &saved));
    assert_eq!(answer.external, None);
    assert_eq!(
        answer.resumed,
        vec![ThreadResume {
            thread: 1,
            external: Some(DELIVERY),
        }]
    );
    assert_eq!(hv.groups().interrupt().paused().thread(1), None);
}

/// lv1's pause step ends with no pause for an unmasked shadow bit
/// [OFW-4.93 lv1 a:0x2C6930-0x2C695C], and the return delivers it.
#[test]
fn a_pause_with_an_unmasked_shadow_bit_returns_at_once_into_the_interrupt() {
    let masked = storage(0);
    let mut hv = booted(&masked);
    let send = call(SEND_EVENT_LOCALLY, &[PORT], 1);
    let saved = msrs(None, Some(KERNEL_MSR));
    answered(hv.dispatch_returning(&send, &masked, &saved));
    let unmasked = storage(PLUG_BIT);
    let pause = call(PAUSE, &[0], 1);
    let answer = answered(hv.dispatch_returning(&pause, &unmasked, &saved));
    assert!(!answer.pauses_caller);
    assert!(answer.changes.is_empty());
    assert_eq!(answer.external, Some(DELIVERY));
    assert_eq!(hv.groups().interrupt().paused().thread(1), None);
}
