//! Group terminate, a thread's exit status, and the argument
//! replacement: each refusal, and the state each success leaves.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::spu::group_join_cause;

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::FakeRuntime;
use crate::host::Lv2Host;
use crate::request::Lv2Request;

const IMG_PTR: u32 = 0x300;
const ARG_PTR: u32 = 0x200;
const NEW_ARG_PTR: u32 = 0x240;
const STATUS_PTR: u32 = 0x600;
const MEM_SIZE: usize = 0x4000;
const GROUP: u32 = 1;
/// The ids of slots 0 and 1 of [`GROUP`].
const THREAD_0: u32 = 0x100;
const THREAD_1: u32 = 0x101;
const UNIT_0: UnitId = UnitId::new(10);
const UNIT_1: UnitId = UnitId::new(11);
const CALLER: UnitId = UnitId::new(0);

const OLD_ARGS: [u64; 4] = [1, 2, 3, 4];
const NEW_ARGS: [u64; 4] = [
    0x1122_3344_5566_7788,
    0x99AA_BBCC_DDEE_FF00,
    0x0102_0304_0506_0708,
    0xF0E0_D0C0_B0A0_9080,
];

fn put(mem: &mut GuestMemory, addr: u32, bytes: &[u8]) {
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(u64::from(addr)), bytes.len() as u64).unwrap(),
        bytes,
    )
    .unwrap();
}

fn arg_bytes(args: [u64; 4]) -> Vec<u8> {
    args.iter().flat_map(|a| a.to_be_bytes()).collect()
}

/// A host holding a two-slot group with both slots initialized, and a
/// runtime whose memory carries the image record and both argument
/// blocks.
fn initialized_group() -> (Lv2Host, FakeRuntime) {
    let mut host = Lv2Host::new();
    host.content_store_mut()
        .register(b"/spu.elf", vec![0xAA, 0xBB]);
    let mut mem = GuestMemory::new(MEM_SIZE);
    put(&mut mem, 0x100, b"/spu.elf\0");
    put(&mut mem, IMG_PTR, &[0, 0, 0, 1, 0, 0, 0, 1]);
    put(&mut mem, ARG_PTR, &arg_bytes(OLD_ARGS));
    put(&mut mem, NEW_ARG_PTR, &arg_bytes(NEW_ARGS));
    let rt = FakeRuntime::with_memory(mem);
    let requests = [
        Lv2Request::SpuImageOpen {
            img_ptr: IMG_PTR,
            path_ptr: 0x100,
        },
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x400,
            num_threads: 2,
            priority: 100,
            attr_ptr: 0,
        },
    ];
    for request in requests {
        assert_eq!(code(&host.dispatch(request, CALLER, &rt)), 0);
    }
    for thread_num in 0..2 {
        let d = host.dispatch(
            Lv2Request::SpuThreadInitialize {
                thread_ptr: 0x500,
                group_id: GROUP,
                thread_num,
                img_ptr: IMG_PTR,
                attr_ptr: 0,
                arg_ptr: ARG_PTR,
            },
            CALLER,
            &rt,
        );
        assert_eq!(code(&d), 0, "initialize slot {thread_num}");
    }
    (host, rt)
}

/// [`initialized_group`], started, with slot 0 on [`UNIT_0`] and slot 1
/// on [`UNIT_1`].
fn running_group() -> (Lv2Host, FakeRuntime) {
    let (mut host, rt) = initialized_group();
    let d = host.dispatch(
        Lv2Request::SpuThreadGroupStart { group_id: GROUP },
        CALLER,
        &rt,
    );
    assert!(
        matches!(d, Lv2Dispatch::RegisterSpu { code: 0, .. }),
        "{d:?}"
    );
    host.record_spu(UNIT_0, GROUP, 0).unwrap();
    host.record_spu(UNIT_1, GROUP, 1).unwrap();
    (host, rt)
}

fn code(d: &Lv2Dispatch) -> u64 {
    match d {
        Lv2Dispatch::Immediate { code, .. } => *code,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

fn terminate(host: &mut Lv2Host, rt: &FakeRuntime, group_id: u32, value: i32) -> Lv2Dispatch {
    host.dispatch(
        Lv2Request::SpuThreadGroupTerminate { group_id, value },
        CALLER,
        rt,
    )
}

fn exit_status(host: &mut Lv2Host, rt: &FakeRuntime, thread_id: u32, ptr: u32) -> Lv2Dispatch {
    host.dispatch(
        Lv2Request::SpuThreadGetExitStatus {
            thread_id,
            status_ptr: ptr,
        },
        CALLER,
        rt,
    )
}

fn set_argument(host: &mut Lv2Host, rt: &FakeRuntime, thread_id: u32, ptr: u32) -> u64 {
    code(&host.dispatch(
        Lv2Request::SpuThreadSetArgument {
            thread_id,
            arg_ptr: ptr,
        },
        CALLER,
        rt,
    ))
}

#[test]
fn terminate_refuses_an_unknown_group_then_a_group_that_is_not_running() {
    let (mut host, rt) = initialized_group();
    assert_eq!(
        code(&terminate(&mut host, &rt, GROUP + 1, 0)),
        errno::CELL_ESRCH.into()
    );
    assert_eq!(
        code(&terminate(&mut host, &rt, GROUP, 0)),
        errno::CELL_ESTAT.into(),
        "a group not yet started"
    );
    assert_eq!(
        host.spu_group_join_result(GROUP),
        None,
        "the refusal changed nothing"
    );
}

#[test]
fn terminate_finishes_the_live_threads_and_a_join_reads_terminated_with_the_value() {
    let (mut host, rt) = running_group();
    // Slot 0 exits first, so only slot 1 is still live.
    assert_eq!(host.spu_thread_exit(UNIT_0, 7), Ok(None));
    let before = host.sync_partial();
    let d = terminate(&mut host, &rt, GROUP, -2);
    assert_eq!(
        d,
        Lv2Dispatch::SpuGroupTerminate {
            group_id: GROUP,
            finished_units: vec![UNIT_1],
        }
    );
    assert_eq!(
        host.spu_group_join_result(GROUP),
        Some((group_join_cause::TERMINATED, (-2i32) as u32))
    );
    assert_eq!(host.live_spu_group(UNIT_1), None, "slot 1 finished");
    assert_ne!(
        host.sync_partial(),
        before,
        "the state hash sees the terminate"
    );
    assert_eq!(
        code(&terminate(&mut host, &rt, GROUP, 0)),
        errno::CELL_ESTAT.into(),
        "a finished group"
    );
}

#[test]
fn exit_status_checks_the_thread_then_the_status_then_the_pointer() {
    let (mut host, rt) = initialized_group();
    assert_eq!(
        code(&exit_status(&mut host, &rt, THREAD_1 + 1, STATUS_PTR)),
        errno::CELL_ESRCH.into(),
        "no thread in that slot"
    );
    assert_eq!(
        code(&exit_status(&mut host, &rt, THREAD_0, 0)),
        errno::CELL_ESTAT.into(),
        "a thread that never ran has no status, and that check comes first"
    );
    let (mut host, rt) = running_group();
    assert_eq!(
        code(&exit_status(&mut host, &rt, THREAD_0, STATUS_PTR)),
        errno::CELL_ESTAT.into(),
        "a running thread"
    );
    host.spu_thread_exit(UNIT_0, 0x7E57).unwrap();
    assert_eq!(
        code(&exit_status(&mut host, &rt, THREAD_0, 0)),
        errno::CELL_EFAULT.into()
    );
    let d = exit_status(&mut host, &rt, THREAD_0, STATUS_PTR);
    let Lv2Dispatch::Immediate { code: 0, effects } = d else {
        panic!("expected a success, got {d:?}");
    };
    let [Effect::SharedWriteIntent { range, bytes, .. }] = effects.as_slice() else {
        panic!("expected one write, got {effects:?}");
    };
    assert_eq!(range.start().raw(), u64::from(STATUS_PTR));
    assert_eq!(bytes.bytes(), &0x7E57u32.to_be_bytes());
}

#[test]
fn a_thread_the_terminate_ended_has_no_exit_status() {
    let (mut host, rt) = running_group();
    host.spu_thread_exit(UNIT_0, 5).unwrap();
    terminate(&mut host, &rt, GROUP, 9);
    assert_eq!(
        code(&exit_status(&mut host, &rt, THREAD_1, STATUS_PTR)),
        errno::CELL_ESTAT.into()
    );
    let d = exit_status(&mut host, &rt, THREAD_0, STATUS_PTR);
    assert!(
        matches!(d, Lv2Dispatch::Immediate { code: 0, .. }),
        "the thread that exited first keeps its status: {d:?}"
    );
}

#[test]
fn set_argument_checks_the_block_then_the_thread() {
    let (mut host, rt) = initialized_group();
    assert_eq!(
        set_argument(&mut host, &rt, THREAD_1 + 1, NEW_ARG_PTR),
        errno::CELL_ESRCH.into()
    );
    assert_eq!(
        set_argument(&mut host, &rt, THREAD_1 + 1, 0),
        errno::CELL_EFAULT.into(),
        "the block check comes first"
    );
    assert_eq!(
        set_argument(&mut host, &rt, THREAD_0, 0),
        errno::CELL_EFAULT.into()
    );
    assert_eq!(
        set_argument(&mut host, &rt, THREAD_0, (MEM_SIZE - 28) as u32),
        errno::CELL_EFAULT.into(),
        "a block that runs four bytes past the mapped end"
    );
}

#[test]
fn set_argument_before_start_is_the_block_the_start_passes() {
    let (mut host, rt) = initialized_group();
    let before = host.sync_partial();
    assert_eq!(set_argument(&mut host, &rt, THREAD_1, NEW_ARG_PTR), 0);
    assert_ne!(host.sync_partial(), before, "the state hash sees the block");
    let d = host.dispatch(
        Lv2Request::SpuThreadGroupStart { group_id: GROUP },
        CALLER,
        &rt,
    );
    let Lv2Dispatch::RegisterSpu { inits, .. } = d else {
        panic!("expected RegisterSpu, got {d:?}");
    };
    assert_eq!(inits[&0].args, OLD_ARGS, "slot 0 keeps its block");
    assert_eq!(inits[&1].args, NEW_ARGS);
}

#[test]
fn set_argument_on_a_running_or_finished_group_is_accepted() {
    let (mut host, rt) = running_group();
    assert_eq!(set_argument(&mut host, &rt, THREAD_0, NEW_ARG_PTR), 0);
    terminate(&mut host, &rt, GROUP, 0);
    assert_eq!(set_argument(&mut host, &rt, THREAD_1, NEW_ARG_PTR), 0);
}
