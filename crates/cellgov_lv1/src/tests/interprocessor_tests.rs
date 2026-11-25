use super::*;
use crate::group::interrupt::bitmap::StatusShadowWrite;
use crate::group::interrupt::InterruptChange;
use crate::return_path::{ExternalDelivery, SavedMsrs};
use crate::{
    HypercallAnswer, HypercallDisposition, HypercallRefusal, HypercallRequest, Hypervisor,
    ThreadInterrupt,
};
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::lv1::call::{
    CONFIGURE_IRQ_STATE_BITMAP, CONNECT_IRQ_PLUG_EXT, CONSTRUCT_EVENT_RECEIVE_PORT,
    SEND_EVENT_LOCALLY,
};
use cellgov_ps3_abi::lv1::interrupt::IRQ_MASK_OFFSET;
use cellgov_time::GuestTicks;

/// The `sc` of the kernel's port call in the setup of a thread's record
/// [OFW-4.93 lv2_kernel a:0x8000000000007d8c]. The console gave thread 1's
/// call the port id 4 (`GAME_OS_PORT_CALLS`).
const RECORD_SC: u64 = 0x8000_0000_0000_7d8c;
const PORT: u64 = 4;

/// The `sc` of the kernel's event send on another thread's port
/// [OFW-4.93 lv2_kernel a:0x8000000000026628].
const SEND_SC: u64 = 0x8000_0000_0002_6628;

const THREAD_1_BITMAP: u64 = 0x0036_9200;

/// Plug 9 is bit 63 - 9 of status doubleword 0 (`plug_status_bit`).
const PLUG: u64 = 9;
const PLUG_BIT: u64 = 1 << 54;

/// Thread 1's pc and MSR at the kernel's send to it, in the 4.93 kernel
/// census: SF, IR, DR and RI, with EE clear.
const TARGET_PC: u64 = 0x8000_0000_0007_e8c4;
const EE_CLEAR_MSR: u64 = 0x8000_0000_0000_0032;

/// The kernel's MSR with EE set.
const EE_SET_MSR: u64 = 0x8000_0000_0000_8032;

/// A decrementer that is not negative.
const DEC: u32 = 0x7FFF_0000;

/// lv1's handler clears bits 48 to 63 of [`EE_SET_MSR`] and keeps SF
/// [OFW-4.93 lv1 a:0x188C-0x18C0], and enters 0x500
/// [OFW-4.93 lv1 a:0x2C3DB8].
const DELIVERY: ExternalRaise = ExternalRaise::Deliver(ExternalDelivery {
    vector: 0x500,
    msr: 0x8000_0000_0000_0000,
});

fn call(number: u64, args: &[u64], thread: Option<u8>) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..args.len()].copy_from_slice(args);
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs,
        pc: SEND_SC,
        lr: 0,
        thread,
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
    let bitmap = call(
        CONFIGURE_IRQ_STATE_BITMAP,
        &[1, 1, THREAD_1_BITMAP],
        Some(1),
    );
    answered(hv.dispatch_with(&bitmap, storage));
    let port = HypercallRequest {
        pc: RECORD_SC,
        ..call(CONSTRUCT_EVENT_RECEIVE_PORT, &[], Some(1))
    };
    answered(hv.dispatch_with(&port, storage));
    let plug = call(CONNECT_IRQ_PLUG_EXT, &[1, 1, PLUG, PORT, 0], Some(0));
    answered(hv.dispatch_with(&plug, storage));
    hv
}

/// Thread 1 running at [`TARGET_PC`] with `msr` and [`DEC`].
fn running(msr: u64) -> RunningThreads {
    RunningThreads([
        None,
        Some(RunningThread {
            pc: TARGET_PC,
            msr,
            decrementer: DEC,
        }),
    ])
}

/// Thread 0's send on thread 1's plug, with thread 1 running with `msr`
/// and both `sc` MSRs at [`EE_SET_MSR`].
fn send_to_running(hv: &mut Hypervisor, storage: &GuestMemory, msr: u64) -> HypercallAnswer {
    let send = call(SEND_EVENT_LOCALLY, &[PORT], Some(0));
    let msrs = SavedMsrs([Some(EE_SET_MSR); 2]);
    answered(hv.dispatch_signalling(&send, storage, &msrs, &running(msr)))
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: r.number,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

/// The census case. lv1 sets the bit in thread 1's shadow and bitmap
/// [OFW-4.93 lv1 a:0x2C6620-0x2C6634], and sends thread 1 an
/// interprocessor interrupt [OFW-4.93 lv1 a:0x2C6720-0x2C672C]. Thread 1
/// runs with EE clear, so lv1's handler sets LPCR\[MER\]
/// [OFW-4.93 lv1 a:0x187C]. The caller's return raises nothing.
#[test]
fn a_send_on_a_running_thread_s_unmasked_plug_signals_it_and_its_handler_sets_mer() {
    let storage = storage(PLUG_BIT);
    let mut hv = booted(&storage);
    let answer = send_to_running(&mut hv, &storage, EE_CLEAR_MSR);
    assert_eq!(answer.r3, 0);
    assert_eq!(answer.outputs(), Some([0; 7]));
    let status = ByteRange::new(GuestAddr::new(THREAD_1_BITMAP), 8).expect("the status");
    assert_eq!(
        answer.effects,
        vec![Effect::SharedWriteIntent {
            range: status,
            bytes: WritePayload::new(PLUG_BIT.to_be_bytes().to_vec()),
            source: UnitId::new(1),
            source_time: GuestTicks::ZERO,
        }]
    );
    let mut expected = HypercallAnswer::success([]);
    expected.push_change(InterruptChange::EventPending(PORT));
    expected.push_change(InterruptChange::StatusShadow(StatusShadowWrite {
        key: thread_1(),
        index: 0,
        value: PLUG_BIT,
    }));
    expected.push_change(InterruptChange::ThreadSignalled(1));
    assert_eq!(answer.changes, expected.changes);
    assert_eq!(answer.external, None);
    assert_eq!(answer.resumed, Vec::new());
    assert_eq!(
        answer.interrupted,
        vec![ThreadInterrupt {
            thread: 1,
            external: Some(ExternalRaise::Mediated),
        }]
    );
    assert_eq!(
        hv.groups().interrupt().bitmaps().shadow(thread_1()),
        [PLUG_BIT, 0, 0, 0]
    );
}

/// With EE set, lv1's handler enters the vector with the guest's SRR0 and
/// SRR1 from HSRR0 and HSRR1 [OFW-4.93 lv1 a:0x188C-0x18C0].
#[test]
fn a_send_on_a_running_thread_with_ee_set_enters_its_vector() {
    let storage = storage(PLUG_BIT);
    let mut hv = booted(&storage);
    let answer = send_to_running(&mut hv, &storage, EE_SET_MSR);
    assert_eq!(
        answer.interrupted,
        vec![ThreadInterrupt {
            thread: 1,
            external: Some(DELIVERY),
        }]
    );
}

/// A masked bit still costs the interrupt, and the handler's shadow test
/// finds no bit [OFW-4.93 lv1 a:0x1A84-0x1AD4].
#[test]
fn a_send_on_a_running_thread_s_masked_plug_signals_it_and_its_handler_raises_nothing() {
    let storage = storage(0);
    let mut hv = booted(&storage);
    let answer = send_to_running(&mut hv, &storage, EE_SET_MSR);
    assert_eq!(
        answer.interrupted,
        vec![ThreadInterrupt {
            thread: 1,
            external: None,
        }]
    );
    assert_eq!(
        hv.groups().interrupt().bitmaps().shadow(thread_1()),
        [PLUG_BIT, 0, 0, 0]
    );
}

#[test]
fn a_send_on_a_thread_that_gives_no_running_registers_is_a_refusal_and_changes_nothing() {
    let storage = storage(PLUG_BIT);
    let mut hv = booted(&storage);
    let before = hv.clone();
    let send = call(SEND_EVENT_LOCALLY, &[PORT], Some(0));
    let msrs = SavedMsrs([Some(EE_SET_MSR); 2]);
    assert_eq!(
        hv.dispatch_signalling(&send, &storage, &msrs, &RunningThreads::default()),
        refused(
            &send,
            "an interprocessor interrupt to a thread whose unit gives no running registers"
        )
    );
    assert_eq!(hv.groups(), before.groups());
}

/// lv1 compares the plug's thread with the thread that runs the call
/// [OFW-4.93 lv1 a:0x2C663C-0x2C664C], which CellGov cannot name here.
#[test]
fn a_send_on_a_running_thread_from_a_unit_with_no_thread_is_a_refusal() {
    let storage = storage(PLUG_BIT);
    let mut hv = booted(&storage);
    let send = call(SEND_EVENT_LOCALLY, &[PORT], None);
    let msrs = SavedMsrs([Some(EE_SET_MSR); 2]);
    assert_eq!(
        hv.dispatch_signalling(&send, &storage, &msrs, &running(EE_CLEAR_MSR)),
        refused(
            &send,
            "an event on a plug of a running thread, from a unit that names no hardware thread"
        )
    );
}

/// The interrupt state after the census send's changes.
fn signalled_state(storage: &GuestMemory) -> InterruptGroupState {
    let mut hv = booted(storage);
    send_to_running(&mut hv, storage, EE_CLEAR_MSR);
    hv.groups().interrupt().clone()
}

fn handler(msr: u64, pc: u64, decrementer: u32) -> Result<Option<ExternalRaise>, UnmodelledError> {
    let storage = storage(PLUG_BIT);
    let after = signalled_state(&storage);
    let running = RunningThread {
        pc,
        msr,
        decrementer,
    };
    handler_raise(&after, thread_1(), &storage, Some(running))
}

/// For a thread in hypervisor state with PR clear, the handler returns
/// with no shadow test [OFW-4.93 lv1 a:0x1A48-0x1A60].
#[test]
fn the_handler_refuses_a_thread_in_hypervisor_state() {
    let hv_msr = EE_SET_MSR | MSR_HV;
    assert_eq!(
        handler(hv_msr, TARGET_PC, DEC),
        Err(UnmodelledError::new(
            "an interprocessor interrupt to a thread in hypervisor state, for which lv1's \
             handler makes no shadow test",
        ))
    );
}

/// lv1's handler tests the decrementer with a signed word compare
/// [OFW-4.93 lv1 a:0x2C632C-0x2C6334], and resets a negative one to
/// 0x7FFFFFFF [OFW-4.93 lv1 a:0x2C636C-0x2C637C].
#[test]
fn the_handler_refuses_a_thread_whose_decrementer_is_negative() {
    let error = Err(UnmodelledError::new(
        "an interprocessor interrupt to a thread whose decrementer is negative, which lv1's \
         handler resets",
    ));
    for decrementer in [0x8000_0000, 0xFFFF_FFFF] {
        assert_eq!(
            handler(EE_SET_MSR, TARGET_PC, decrementer),
            error,
            "decrementer {decrementer:#x}"
        );
    }
}

/// The compare branches only on less than zero, so a decrementer of 0 is
/// not negative [OFW-4.93 lv1 a:0x2C632C-0x2C6334].
#[test]
fn the_handler_enters_the_vector_of_a_thread_whose_decrementer_is_0_or_0x7fffffff() {
    for decrementer in [0, 0x7FFF_FFFF] {
        assert_eq!(
            handler(EE_SET_MSR, TARGET_PC, decrementer),
            Ok(Some(DELIVERY)),
            "decrementer {decrementer:#x}"
        );
    }
}

/// lv1 enters no vector for a saved pc of 0x28B8
/// [OFW-4.93 lv1 a:0x188C-0x18A8]. With EE clear, it sets LPCR\[MER\]
/// first [OFW-4.93 lv1 a:0x187C].
#[test]
fn the_handler_refuses_a_thread_with_ee_set_at_lv1_s_trampoline_slot() {
    assert_eq!(
        handler(EE_SET_MSR, MEDIATED_UNDELIVERED_PC, DEC),
        Err(UnmodelledError::new(
            "an interprocessor interrupt to a thread at lv1's trampoline slot 0x28B8, for which \
             lv1's handler enters no vector",
        ))
    );
    assert_eq!(
        handler(EE_CLEAR_MSR, MEDIATED_UNDELIVERED_PC, DEC),
        Ok(Some(ExternalRaise::Mediated))
    );
}

/// A second store to the generation port at the same priority merges
/// with the pending interrupt [CBE-Handbook p:268 s:9.6.2.4], so a call
/// that signals one thread twice gives it one interrupt.
#[test]
fn two_signals_of_one_thread_in_one_call_give_it_one_interrupt() {
    let storage = storage(PLUG_BIT);
    let hv = booted(&storage);
    let send = call(SEND_EVENT_LOCALLY, &[PORT], Some(0));
    let mut answer = HypercallAnswer::success([]);
    answer.push_change(InterruptChange::StatusShadow(StatusShadowWrite {
        key: thread_1(),
        index: 0,
        value: PLUG_BIT,
    }));
    answer.push_change(InterruptChange::ThreadSignalled(1));
    answer.push_change(InterruptChange::ThreadSignalled(1));
    let msrs = SavedMsrs([Some(EE_SET_MSR); 2]);
    let running = running(EE_CLEAR_MSR);
    let returned = answered(crate::external::returns(
        &send,
        Some("4.93"),
        hv.groups(),
        &storage,
        &msrs,
        &running,
        answer,
    ));
    assert_eq!(
        returned.interrupted,
        vec![ThreadInterrupt {
            thread: 1,
            external: Some(ExternalRaise::Mediated),
        }]
    );
}

#[test]
fn a_signal_from_a_call_whose_return_makes_no_shadow_test_is_a_refusal() {
    let storage = storage(PLUG_BIT);
    let hv = booted(&storage);
    let send = call(SEND_EVENT_LOCALLY, &[PORT], Some(0));
    let mut answer = HypercallAnswer::success([]);
    answer.push_change(InterruptChange::ThreadSignalled(1));
    let msrs = SavedMsrs([Some(EE_SET_MSR); 2]);
    let running = running(EE_CLEAR_MSR);
    assert_eq!(
        crate::external::returns(&send, None, hv.groups(), &storage, &msrs, &running, answer),
        refused(
            &send,
            "an interprocessor interrupt from a call whose return makes no shadow test"
        )
    );
}
