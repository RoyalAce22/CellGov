use crate::return_path::{ExternalRaise, SavedMsrs};
use crate::{
    HypercallAnswer, HypercallDisposition, HypercallRefusal, HypercallRequest, Hypervisor,
    MediatedEntry, MediatedTake,
};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::ppu_interrupt::SRR1_MEDIATED_EXTERNAL;
use cellgov_ps3_abi::hw::ppu_supervisor::MSR_HV;
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

/// The `sc` of the kernel's event send
/// [OFW-4.93 lv2_kernel a:0x8000000000025a34].
const SEND_SC: u64 = 0x8000_0000_0002_5a34;

const THREAD_1_BITMAP: u64 = 0x0036_9200;

/// Plug 9 is bit 63 - 9 of status doubleword 0 (`plug_status_bit`).
const PLUG: u64 = 9;
const PLUG_BIT: u64 = 1 << 54;

/// The 4.93 kernel's MSR in its external interrupt dispatch, with EE
/// clear: SF, IR and DR.
const DISPATCH_MSR: u64 = 0x8000_0000_0000_0030;

/// The kernel's MSR after it sets EE and RI again.
const TAKE_MSR: u64 = 0x8000_0000_0000_8032;

/// The kernel's pc after its mask update hypercall
/// [OFW-4.93 lv2_kernel a:0x8000000000008e58].
const TAKE_PC: u64 = 0x8000_0000_0000_8e58;

const UNIT: UnitId = UnitId::new(1);

/// A decrementer that is not negative.
const DEC: u32 = 0x0010_0000;

fn call(number: u64, args: &[u64], thread: u8) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..args.len()].copy_from_slice(args);
    HypercallRequest {
        unit: UNIT,
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

/// A 4.93 hypervisor whose thread 1 raised its own plug with EE clear, so
/// its return set LPCR\[MER\].
fn requested() -> Hypervisor {
    let storage = storage(PLUG_BIT);
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let bitmap = call(CONFIGURE_IRQ_STATE_BITMAP, &[1, 1, THREAD_1_BITMAP], 1);
    answered(hv.dispatch_with(&bitmap, &storage));
    let port = HypercallRequest {
        pc: RECORD_SC,
        ..call(CONSTRUCT_EVENT_RECEIVE_PORT, &[], 1)
    };
    answered(hv.dispatch_with(&port, &storage));
    let plug = call(CONNECT_IRQ_PLUG_EXT, &[1, 1, PLUG, PORT, 0], 0);
    answered(hv.dispatch_with(&plug, &storage));
    let send = call(SEND_EVENT_LOCALLY, &[PORT], 1);
    let msrs = SavedMsrs([None, Some(DISPATCH_MSR)]);
    let answer = answered(hv.dispatch_returning(&send, &storage, &msrs));
    assert_eq!(answer.external, Some(ExternalRaise::Mediated));
    hv
}

fn take(srr0: u64, msr: u64) -> MediatedTake {
    MediatedTake {
        unit: UNIT,
        thread: Some(1),
        srr0,
        srr1: msr | SRR1_MEDIATED_EXTERNAL,
        decrementer: Some(DEC),
    }
}

fn refusal(pc: u64, what: &'static str) -> HypercallRefusal {
    HypercallRefusal::MediatedExternal {
        unit: UNIT,
        pc,
        what,
    }
}

/// lv1's handler finds the bit again and enters the vector
/// [OFW-4.93 lv1 a:0x1A84-0x1AD4] [OFW-4.93 lv1 a:0x188C-0x18C0].
#[test]
fn a_take_whose_shadow_and_mask_share_a_bit_is_answered() {
    let mut hv = requested();
    assert_eq!(
        hv.take_mediated_external(&take(TAKE_PC, TAKE_MSR), &storage(PLUG_BIT)),
        Ok(MediatedEntry::Vector)
    );
    assert_eq!(hv.refusal(), None);
}

#[test]
fn a_take_refusal_is_kept_and_a_later_one_does_not_replace_it() {
    let mut hv = requested();
    let expected = refusal(
        0x28B8,
        "a take at lv1's trampoline slot 0x28B8, for which lv1's handler enters no vector",
    );
    assert_eq!(
        hv.take_mediated_external(&take(0x28B8, TAKE_MSR), &storage(PLUG_BIT)),
        Err(expected)
    );
    assert_eq!(hv.refusal(), Some(expected));
    let second = refusal(
        TAKE_PC,
        "a take whose MSR holds a bit of 48 to 63 that lv1 clears and the PPU's interrupt \
         entry keeps",
    );
    assert_eq!(
        hv.take_mediated_external(&take(TAKE_PC, TAKE_MSR | 0x80), &storage(PLUG_BIT)),
        Err(second)
    );
    assert_eq!(hv.refusal(), Some(expected));
}

#[test]
fn a_take_refusal_names_no_hypercall_number() {
    let mut hv = requested();
    let Err(refused) = hv.take_mediated_external(&take(0x28B8, TAKE_MSR), &storage(PLUG_BIT))
    else {
        panic!("a take at the trampoline slot is answered");
    };
    assert_eq!(refused.number(), None);
}

#[test]
fn a_take_at_a_firmware_with_no_logical_ppe_id_is_a_refusal() {
    let mut hv = Hypervisor::new();
    assert_eq!(
        hv.take_mediated_external(&take(TAKE_PC, TAKE_MSR), &storage(PLUG_BIT)),
        Err(refusal(
            TAKE_PC,
            "a take at a firmware whose logical PPE id no table gives",
        ))
    );
}

#[test]
fn a_take_on_a_unit_with_no_hardware_thread_is_a_refusal() {
    let mut hv = requested();
    let threadless = MediatedTake {
        thread: None,
        ..take(TAKE_PC, TAKE_MSR)
    };
    assert_eq!(
        hv.take_mediated_external(&threadless, &storage(PLUG_BIT)),
        Err(refusal(
            TAKE_PC,
            "a take on a unit that runs no PPE hardware thread",
        ))
    );
}

/// lv1 keeps LPCR\[MER\] for a saved pc of 0x28B8
/// [OFW-4.93 lv1 a:0x188C-0x18A8].
#[test]
fn a_take_at_lv1_s_trampoline_slot_is_a_refusal() {
    let mut hv = requested();
    assert_eq!(
        hv.take_mediated_external(&take(0x28B8, TAKE_MSR), &storage(PLUG_BIT)),
        Err(refusal(
            0x28B8,
            "a take at lv1's trampoline slot 0x28B8, for which lv1's handler enters no vector",
        ))
    );
}

/// lv1 clears bit 56 in the vector's MSR [OFW-4.93 lv1 a:0x1898], and the
/// PPU's interrupt entry keeps it [CBE-Handbook p:292 s:9.12].
#[test]
fn a_take_whose_msr_holds_a_bit_that_lv1_clears_is_a_refusal() {
    let mut hv = requested();
    assert_eq!(
        hv.take_mediated_external(&take(TAKE_PC, TAKE_MSR | 0x80), &storage(PLUG_BIT)),
        Err(refusal(
            TAKE_PC,
            "a take whose MSR holds a bit of 48 to 63 that lv1 clears and the PPU's interrupt \
             entry keeps",
        ))
    );
}

/// Hypervisor state is `MSR[HV]` = 1 with `MSR[PR]` = 0
/// [PPC-Book3 p:10 s:2.2.3]. [`TAKE_MSR`] holds PR = 0.
#[test]
fn a_take_in_hypervisor_state_is_a_refusal() {
    let mut hv = requested();
    let msr = TAKE_MSR | MSR_HV;
    assert_eq!(
        hv.take_mediated_external(&take(TAKE_PC, msr), &storage(PLUG_BIT)),
        Err(refusal(
            TAKE_PC,
            "a take in hypervisor state, which lv1 treats as an interrupt of its own",
        ))
    );
}

#[path = "mediated_decrementer_tests.rs"]
mod decrementer;

#[path = "mediated_resume_tests.rs"]
mod resume;
