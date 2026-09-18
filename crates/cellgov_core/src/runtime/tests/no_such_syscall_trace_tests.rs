//! Runtime trace classification for out-of-table syscalls.

use super::*;
use cellgov_ps3_abi::lv2::syscall::SYSCALL_TABLE_SLOTS;
use cellgov_trace::{TraceReader, TraceRecord, TracedSyscallDisposition};

#[test]
fn an_out_of_table_syscall_has_its_own_trace_disposition() {
    let mut rt = build(4096, 4, 100);
    let mut args = [0u64; 9];
    args[0] = SYSCALL_TABLE_SLOTS;
    rt.registry_mut().register_with(|id| Lv2SyscallEmitterUnit {
        id,
        steps: Cell::new(0),
        syscall_args: args,
    });
    let step = rt.step().unwrap();
    rt.commit_step(&step.result, &step.effects).unwrap();

    let disposition = TraceReader::new(rt.trace().bytes())
        .map(|record| record.expect("decode"))
        .find_map(|record| match record {
            TraceRecord::SyscallEntered { disposition, .. } => Some(disposition),
            _ => None,
        })
        .expect("syscall entry");
    assert_eq!(disposition, TracedSyscallDisposition::NoSuchSyscall);
}
