//! The runtime reports an SPU's inbound-mailbox occupancy, and `rchcnt`
//! on `SPU_RdInMbox` reads it.

use cellgov_core::Runtime;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{SPU_IN_MBOX_DEPTH, SPU_RD_IN_MBOX};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

// [CBEA p:135 s:9.5.3] SPU_RdInMbox counts the messages in the inbound mailbox.
#[test]
fn rchcnt_reads_the_messages_waiting_in_the_units_mailbox() {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(100), 100);
    let mailbox = rt
        .mailbox_registry_mut()
        .register(SPU_IN_MBOX_DEPTH as usize);
    for message in [7, 8, 9] {
        rt.mailbox_registry_mut()
            .get_mut(mailbox)
            .expect("registered mailbox")
            .force_send(message);
    }
    // `rchcnt r3, SPU_RdInMbox` then `stop`.
    let program = [((0x00F << 21) | (u32::from(SPU_RD_IN_MBOX) << 7) | 3), 0];
    let unit = rt.register_unit_with(|id| {
        assert_eq!(
            id.raw(),
            mailbox.raw(),
            "the unit reads the mailbox sharing its id"
        );
        let mut spu = SpuExecutionUnit::new(id);
        for (i, word) in program.iter().enumerate() {
            spu.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        spu
    });

    let step = rt.step().expect("the unit runs");
    rt.commit_step(&step.result, &step.effects)
        .expect("the step commits");

    let spu = rt
        .registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("the SPU unit");
    assert_eq!(spu.state().reg_word(3), 3);
}
