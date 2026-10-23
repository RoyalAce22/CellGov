//! The runtime's service of an SPU thread's interrupt mailbox:
//! - the group start connects the mailbox;
//! - a send wakes the PPU parked on the port's queue and answers in the
//!   SPU's inbound mailbox;
//! - a word LV2 refuses stops the thread with a named failure.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, FakeIsaUnit, LocalDiagnostics,
    OutMailboxes, ProblemStateError, StopRegisters, UnitStatus, YieldReason,
};
use cellgov_lv2::{Lv2Request, SpuThreadError};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::SPU_STATUS_R;
use cellgov_ps3_abi::lv2::spu::{event, user_event};
use cellgov_sync::MailboxId;
use cellgov_time::{Budget, InstructionCost};

use crate::runtime::types::RuntimeMode;
use crate::runtime::Runtime;

const PATH_PTR: u32 = 0x100;
const QUEUE_ID_PTR: u32 = 0x200;
const IMG_PTR: u32 = 0x300;
const GROUP_ID_PTR: u32 = 0x400;
const THREAD_ID_PTR: u32 = 0x500;
const PORT: u32 = 3;
const DATA3: u32 = 0xCAFE_BABE;

/// An SPU stand-in at the moment it wrote both mailboxes: the word in
/// `SPU_WrOutIntrMbox`, the data in `SPU_WrOutMbox`, and its interrupt.
#[derive(Clone)]
struct MailboxUnit {
    inner: FakeIsaUnit,
    connected: bool,
    interrupt: bool,
    intr_mbox: Option<u32>,
    out_mbox: Option<u32>,
    stopped: bool,
}

impl ExecutionUnit for MailboxUnit {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.inner.unit_id()
    }

    fn status(&self) -> UnitStatus {
        UnitStatus::Runnable
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.inner.run_until_yield(budget, ctx, effects)
    }

    fn snapshot(&self) {}

    fn spu_status(&self) -> Option<u32> {
        Some(if self.stopped { 0 } else { SPU_STATUS_R })
    }

    fn stop_registers(&self) -> Option<StopRegisters> {
        self.stopped.then_some(StopRegisters {
            status: 0,
            npc: 0x40,
        })
    }

    fn request_stop(&mut self, _waiting: bool) -> Result<(), ProblemStateError> {
        self.stopped = true;
        Ok(())
    }

    fn connect_out_intr_mbox(&mut self) -> Result<(), ProblemStateError> {
        self.connected = true;
        Ok(())
    }

    fn read_out_intr_mbox(&mut self) -> Result<Option<u32>, ProblemStateError> {
        Ok(self.intr_mbox.take())
    }

    fn take_out_intr_mbox_interrupt(&mut self) -> bool {
        std::mem::take(&mut self.interrupt)
    }

    fn read_out_mbox(&mut self) -> Result<Option<u32>, ProblemStateError> {
        Ok(self.out_mbox.take())
    }

    fn out_mailboxes(&self) -> Option<OutMailboxes> {
        Some(OutMailboxes {
            out_mbox: self.out_mbox,
            out_intr_mbox: self.intr_mbox,
        })
    }
}

fn word_at(rt: &Runtime, addr: u32) -> u32 {
    let mem = rt.memory().as_bytes();
    let a = addr as usize;
    u32::from_be_bytes([mem[a], mem[a + 1], mem[a + 2], mem[a + 3]])
}

/// A started one-thread group whose thread connects port [`PORT`] to a
/// one-entry queue and whose SPU wrote [`DATA3`], then `word`.
/// Returns the runtime, the SPU, the PPU caller and the queue id.
fn started(word: u32) -> (Runtime, UnitId, UnitId, u32) {
    started_with(word, true)
}

/// [`started`], where `interrupt` says whether the SPU raised its
/// mailbox interrupt.
fn started_with(word: u32, interrupt: bool) -> (Runtime, UnitId, UnitId, u32) {
    let mut mem = GuestMemory::new(0x1000);
    for (addr, bytes) in [
        (PATH_PTR, &b"/spu.elf\0"[..]),
        (IMG_PTR, &[0, 0, 0, 1, 0, 0, 0, 1][..]),
    ] {
        mem.apply_commit(
            ByteRange::new(GuestAddr::new(u64::from(addr)), bytes.len() as u64).expect("range"),
            bytes,
        )
        .expect("the setup bytes");
    }
    let mut rt = Runtime::new(mem, Budget::new(1), 10);
    rt.lv2_host_mut()
        .content_store_mut()
        .register(b"/spu.elf", vec![0xAA, 0xBB]);
    rt.set_spu_factory(move |id, _| {
        Ok(Box::new(MailboxUnit {
            inner: FakeIsaUnit::new(id, vec![]),
            connected: false,
            interrupt,
            intr_mbox: Some(word),
            out_mbox: Some(DATA3),
            stopped: false,
        }))
    });
    let caller = rt.register_unit_with(|id| FakeIsaUnit::new(id, vec![]));
    rt.lv2_host_mut().seed_primary_ppu_thread(
        caller,
        cellgov_lv2::ppu_thread::PpuThreadAttrs {
            entry: 0x100,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x1000,
            priority: 1000,
            tls_base: 0,
        },
    );
    let call = |rt: &mut Runtime, request: Lv2Request| {
        rt.dispatch_lv2_request(request, caller);
        let code = rt.registry.drain_syscall_return(caller);
        assert_eq!(code, Some(0), "the setup call answers OK");
    };
    call(
        &mut rt,
        Lv2Request::SpuImageOpen {
            img_ptr: IMG_PTR,
            path_ptr: PATH_PTR,
        },
    );
    call(
        &mut rt,
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: GROUP_ID_PTR,
            num_threads: 1,
            priority: 100,
            attr_ptr: 0,
        },
    );
    call(
        &mut rt,
        Lv2Request::EventQueueCreate {
            id_ptr: QUEUE_ID_PTR,
            attr_ptr: 0,
            key: 0,
            size: 1,
        },
    );
    let group = word_at(&rt, GROUP_ID_PTR);
    let queue = word_at(&rt, QUEUE_ID_PTR);
    call(
        &mut rt,
        Lv2Request::SpuThreadInitialize {
            thread_ptr: THREAD_ID_PTR,
            group_id: group,
            thread_num: 0,
            img_ptr: IMG_PTR,
            attr_ptr: 0,
            arg_ptr: 0,
        },
    );
    let thread = word_at(&rt, THREAD_ID_PTR);
    call(
        &mut rt,
        Lv2Request::SpuThreadConnectEvent {
            thread_id: thread,
            queue_id: queue,
            event_type: event::THREAD_USER,
            port: PORT,
        },
    );
    call(&mut rt, Lv2Request::SpuThreadGroupStart { group_id: group });
    let spu = rt
        .lv2_host()
        .thread_groups()
        .unit_for_thread(thread)
        .expect("the start registered the SPU");
    (rt, spu, caller, queue)
}

fn mailbox_unit(rt: &Runtime, unit: UnitId) -> &MailboxUnit {
    rt.registry
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<MailboxUnit>())
        .expect("a MailboxUnit")
}

fn inbound(rt: &mut Runtime, unit: UnitId) -> Option<u32> {
    rt.mailbox_registry
        .get_mut(MailboxId::new(unit.raw()))
        .expect("the SPU's inbound mailbox")
        .try_receive()
}

#[test]
fn a_send_wakes_the_parked_receiver_and_answers_ok_in_the_inbound_mailbox() {
    let (mut rt, spu, caller, queue) = started(PORT << user_event::PORT_SHIFT | 0x12_3456);
    assert!(
        mailbox_unit(&rt, spu).connected,
        "the start makes LV2 the interrupt mailbox's reader"
    );
    rt.dispatch_lv2_request(
        Lv2Request::EventQueueReceive {
            queue_id: queue,
            out_ptr: 0x600,
            timeout: 0,
        },
        caller,
    );
    assert_eq!(
        rt.registry.effective_status(caller),
        Some(UnitStatus::Blocked)
    );

    rt.serve_spu_thread_interrupt_mailbox(spu);
    assert_eq!(
        rt.registry.effective_status(caller),
        Some(UnitStatus::Runnable),
        "the event woke the receiver"
    );
    assert_eq!(inbound(&mut rt, spu), Some(0), "CELL_OK");
    assert_eq!(inbound(&mut rt, spu), None, "one answer");
    let unit = mailbox_unit(&rt, spu);
    assert_eq!((unit.intr_mbox, unit.out_mbox), (None, None), "both read");
    assert!(rt.take_spu_thread_failure().is_none());
}

#[test]
fn a_throw_with_no_receiver_queues_the_event_and_answers_nothing() {
    let (mut rt, spu, _, queue) =
        started(user_event::THROW | PORT << user_event::PORT_SHIFT | 0x65_4321);
    rt.serve_spu_thread_interrupt_mailbox(spu);
    assert_eq!(inbound(&mut rt, spu), None, "a throw reads no answer");
    let queued = rt
        .lv2_host()
        .event_queues()
        .lookup(queue)
        .expect("the queue")
        .len();
    assert_eq!(queued, 1);
}

#[test]
fn a_word_lv2_refuses_stops_the_thread_with_a_named_failure() {
    let word = user_event::OTHER_SERVICE | 1;
    let (mut rt, spu, _, _) = started(word);
    rt.serve_spu_thread_interrupt_mailbox(spu);
    assert_eq!(rt.registry.effective_status(spu), Some(UnitStatus::Faulted));
    assert!(mailbox_unit(&rt, spu).stopped, "the SPU stopped");
    assert_eq!(
        rt.take_spu_thread_failure().map(|f| (f.unit, f.error)),
        Some((spu, SpuThreadError::UnservedInterruptMailbox(word)))
    );
}

#[test]
fn a_fault_driven_step_that_ends_after_a_throw_serves_it() {
    let (mut rt, spu, _, queue) = started(user_event::THROW | PORT << user_event::PORT_SHIFT);
    rt.set_mode(RuntimeMode::FaultDriven);
    // Every other fast-path condition holds, so only the word in the
    // interrupt mailbox sends the commit down the slow path.
    assert!(rt.dma_queue.is_empty());
    assert_eq!(rt.timer_wakes_pending(), 0);
    rt.last_scheduled_unit = Some(spu);
    let result = ExecutionStepResult {
        yield_reason: YieldReason::BudgetExhausted,
        consumed_cost: InstructionCost::ONE,
        local_diagnostics: LocalDiagnostics::empty(),
        fault: None,
        syscall_args: None,
    };
    rt.commit_unchecked(&result, &[]).expect("the commit");
    let queued = rt
        .lv2_host()
        .event_queues()
        .lookup(queue)
        .expect("the queue")
        .len();
    assert_eq!(queued, 1, "the commit served the throw");
}

#[test]
fn a_step_without_an_interrupt_serves_nothing() {
    // The word is in the mailbox, but no write raised the interrupt.
    let (mut rt, spu, _, queue) = started_with(PORT << user_event::PORT_SHIFT, false);
    rt.serve_spu_thread_interrupt_mailbox(spu);
    let queued = rt
        .lv2_host()
        .event_queues()
        .lookup(queue)
        .expect("the queue")
        .len();
    assert_eq!(queued, 0, "no interrupt, no event");
    assert_eq!(inbound(&mut rt, spu), None, "no answer");
    let unit = mailbox_unit(&rt, spu);
    assert_eq!(
        (unit.intr_mbox, unit.out_mbox),
        (Some(PORT << user_event::PORT_SHIFT), Some(DATA3)),
        "neither mailbox read"
    );
}
