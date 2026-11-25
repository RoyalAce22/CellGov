use super::*;
use crate::group::state::StateChange;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::OPEN_DEVICE;
use cellgov_time::GuestTicks;

/// The `sc` of the kernel's `open_device` of a storage device
/// [OFW-4.93 lv2_kernel a:0x80000000000ac120].
const KERNEL_OPEN_SC: u64 = 0x8000_0000_000A_C120;

/// The `sc` of the kernel's device command
/// [OFW-4.93 lv2_kernel a:0x80000000000ab390].
const KERNEL_COMMAND_SC: u64 = 0x8000_0000_000A_B390;

/// The r5 of the kernel's open of a storage device
/// [OFW-4.93 lv2_kernel a:0x80000000000ac0fc-0x80000000000ac120].
const KERNEL_OPEN_R5: u64 = 0x8000_0000_0000_0000;

/// The data buffer of the kernel's first command, in the real mode
/// region.
const DATA: u64 = 0x4D_0160;

/// The bytes of the caller's real address space in these tests.
const MEMORY_BYTES: usize = 0x50_0000;

/// The r3 to r8 of the 4.93 kernel's first device command in CellGov's
/// run of the kernel from its entry: device 2, the controller query, a
/// 0x40-byte command buffer and a 12-byte data buffer.
const KERNEL_QUERY: [u64; 6] = [2, 0x1A, 0x4D_0170, 0x40, DATA, 0xC];

fn call(number: u64, pc: u64, args: &[u64]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..args.len()].copy_from_slice(args);
    HypercallRequest {
        unit: UnitId::new(0),
        number,
        inputs,
        pc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(41_728),
    }
}

fn command(args: &[u64]) -> HypercallRequest {
    call(STORAGE_SEND_DEVICE_COMMAND, KERNEL_COMMAND_SC, args)
}

/// Memory whose data buffer holds nonzero bytes.
fn memory() -> GuestMemory {
    let mut memory = GuestMemory::new(MEMORY_BYTES);
    let range = ByteRange::new(GuestAddr::new(DATA), 12).expect("the data words");
    memory
        .apply_commit(range, &[0xA5; 12])
        .expect("the data write");
    memory
}

/// A 4.93 hypervisor after the kernel's opens of each device of
/// `devices`.
fn opened(devices: &[u64], memory: &GuestMemory) -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    for &device in devices {
        let open = call(OPEN_DEVICE, KERNEL_OPEN_SC, &[4, device, KERNEL_OPEN_R5]);
        assert_eq!(answered(hv.dispatch_with(&open, memory)).r3, 0, "open");
    }
    hv
}

fn answered(disposition: HypercallDisposition) -> HypercallAnswer {
    match disposition {
        HypercallDisposition::Answered(answer) => answer,
        refused => panic!("{refused:?}"),
    }
}

/// lv1's `status` in r3, zero-extended as the handler returns it
/// [OFW-4.93 lv1 a:0x309634], with r4 to r10 cleared.
fn status(status: i64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(zero_extended(status), []))
}

/// The request function stores the lowest free tag, 0 for a device that
/// holds no tag, in the return slot of r4
/// [OFW-4.93 lv1 a:0x25398C-0x253A04]. The request method returns 0 when
/// the command method succeeds [OFW-4.93 lv1 a:0x256E10-0x256E58]. The
/// query function stores 0 in each of the three words of the data buffer
/// [OFW-4.93 lv1 a:0x25F984-0x25F98C].
#[test]
fn the_kernel_query_gets_tag_0_zeroes_the_three_words_and_waits() {
    let memory = memory();
    let mut hv = opened(&[2], &memory);
    let answer = answered(hv.dispatch_with(&command(&KERNEL_QUERY), &memory));
    assert_eq!(answer.r3, 0);
    assert_eq!(answer.outputs(), Some([0, 0, 0, 0, 0, 0, 0]));
    let range = ByteRange::new(GuestAddr::new(DATA), 12).expect("the data words");
    assert_eq!(
        answer.effects,
        vec![Effect::SharedWriteIntent {
            range,
            bytes: WritePayload::new(vec![0; 12]),
            source: UnitId::new(0),
            source_time: GuestTicks::new(41_728),
        }]
    );
    let started = DeviceCommand {
        device: 2,
        tag: 0,
        command: 0x1A,
        data: DATA,
        data_bytes: 12,
    };
    assert_eq!(
        answer.changes,
        vec![StateChange::from(StorageChange::DeviceCommandStarted(
            started
        ))]
    );
    assert_eq!(
        hv.groups().storage().device_commands().collect::<Vec<_>>(),
        [started]
    );
}

#[test]
fn a_query_while_a_command_of_the_device_waits_is_a_refusal() {
    let memory = memory();
    let mut hv = opened(&[2], &memory);
    answered(hv.dispatch_with(&command(&KERNEL_QUERY), &memory));
    assert_eq!(hv.groups().storage().device_held_tags(2), 0b1);
    let request = command(&KERNEL_QUERY);
    assert_eq!(
        hv.dispatch_with(&request, &memory),
        QUEUED.refusal(&request)
    );
}

/// The request function stores tag 1 in the return slot of r4
/// [OFW-4.93 lv1 a:0x253A04]. The dispatcher refuses command id 3
/// [OFW-4.93 lv1 a:0x272CAC], and the request method frees the tag with
/// no write to the slot [OFW-4.93 lv1 a:0x256E5C-0x256EF0].
#[test]
fn an_unaccepted_command_while_tag_0_is_held_gets_minus_17_with_tag_1_in_r4() {
    let memory = memory();
    let mut hv = opened(&[2], &memory);
    answered(hv.dispatch_with(&command(&KERNEL_QUERY), &memory));
    let mut inputs = KERNEL_QUERY;
    inputs[1] = 3;
    assert_eq!(
        hv.dispatch_with(&command(&inputs), &memory),
        HypercallDisposition::Answered(HypercallAnswer::lv1_status(
            zero_extended(SEND_COMMAND_BAD_REQUEST_STATUS),
            [1],
        ))
    );
    assert_eq!(hv.groups().storage().device_held_tags(2), 0b1);
}

/// [OFW-4.93 lv1 a:0x251CD8] gives -6, [OFW-4.93 lv1 a:0x251C98] -11 and
/// [OFW-4.93 lv1 a:0x272CAC] -17. The registration capture reads no
/// object for devices 5 and 6.
#[test]
fn absent_unopened_and_unaccepted_commands_get_lv1_statuses() {
    let memory = memory();
    let mut hv = opened(&[2], &memory);
    let cases = [
        (5, 0x1A, SEND_COMMAND_NO_DEVICE_STATUS),
        (7, 0x1A, SEND_COMMAND_NO_DEVICE_STATUS),
        (3, 0x1A, SEND_COMMAND_NOT_READY_STATUS),
        (u64::MAX, 0x1A, SEND_COMMAND_NOT_READY_STATUS),
        (2, 3, SEND_COMMAND_BAD_REQUEST_STATUS),
        (2, 0x31, SEND_COMMAND_BAD_REQUEST_STATUS),
    ];
    for (device, id, expected) in cases {
        let mut inputs = KERNEL_QUERY;
        inputs[0] = device;
        inputs[1] = id;
        assert_eq!(
            hv.dispatch_with(&command(&inputs), &memory),
            status(expected),
            "device {device} command {id:#x}"
        );
    }
    assert_eq!(hv.groups().storage().device_commands().count(), 0);
}

/// The request function checks the data buffer for the full r8 before
/// the dispatcher reads the command id [OFW-4.93 lv1 a:0x2539D0-0x2539D4].
#[test]
fn a_data_buffer_past_its_region_is_a_refusal_before_the_command_id() {
    let memory = memory();
    let mut hv = opened(&[2], &memory);
    let rm_size = real_mode_size("4.93").expect("the 4.93 real mode size");
    let cases = [
        [2, 3, 0x4D_0170, 0x40, rm_size - 4, 0xC],
        [2, 3, 0x4D_0170, 0x40, DATA, 0x1_0000_000C],
        [2, 0x1A, 0x4D_0170, 0x40, DATA, 0x1_0000_000C],
    ];
    for inputs in cases {
        let request = command(&inputs);
        assert_eq!(
            hv.dispatch_with(&request, &memory),
            DATA_BUFFER.refusal(&request),
            "{inputs:x?}"
        );
    }
    assert_eq!(hv.groups().storage().device_commands().count(), 0);
}

/// CellGov holds no storage for a command buffer that ends past the real
/// mode region. A command length of 0 leaves the command buffer
/// untranslated, so its address has no effect.
#[test]
fn a_command_buffer_with_no_storage_is_a_refusal_and_one_of_no_bytes_is_not_translated() {
    let memory = memory();
    let mut hv = opened(&[2], &memory);
    let rm_size = real_mode_size("4.93").expect("the 4.93 real mode size");
    let request = command(&[2, 0x1A, rm_size - 4, 0x40, DATA, 0xC]);
    assert_eq!(
        hv.dispatch_with(&request, &memory),
        COMMAND_BUFFER.refusal(&request)
    );
    assert_eq!(hv.groups().storage().device_commands().count(), 0);
    let empty = command(&[2, 0x1A, u64::MAX, 0, DATA, 0xC]);
    assert_eq!(answered(hv.dispatch_with(&empty, &memory)).r3, 0);
    assert_eq!(hv.groups().storage().device_held_tags(2), 0b1);
}

#[test]
fn inputs_outside_the_kernel_query_are_refusals() {
    let memory = memory();
    let mut hv = opened(&[2, 3], &memory);
    let cases = [
        ([3, 0x1A, 0x4D_0170, 0x40, DATA, 0xC], DEVICE_METHOD),
        ([2, 0x10, 0x4D_0170, 0x40, DATA, 0xC], OTHER_COMMAND),
        ([2, 0x1A, 0x4D_0170, 0x40, DATA, 0x8], SHORT_DATA),
        ([2, 0x1A, 0x4D_0170, 0x40, DATA, 0], SHORT_DATA),
    ];
    for (inputs, refusal) in cases {
        let request = command(&inputs);
        assert_eq!(
            hv.dispatch_with(&request, &memory),
            refusal.refusal(&request),
            "{inputs:x?}"
        );
    }
    let mut hv = opened(&[2], &memory);
    hv.set_firmware("4.92");
    let request = command(&KERNEL_QUERY);
    assert_eq!(
        hv.dispatch_with(&request, &memory),
        UNREAD_FIRMWARE.refusal(&request)
    );
}
