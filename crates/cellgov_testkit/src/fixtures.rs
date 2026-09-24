//! Pre-built scenario fixtures plus the [`ScenarioFixture`] value object
//! feeding the runner.
//!
//! A fixture carries runtime construction inputs (memory size, per-step
//! budget, max-steps cap) plus one-shot callbacks for seeding memory and
//! registering units. The runner consumes the fixture; tests never touch
//! `Runtime` directly.
//!
//! # Examples
//!
//! ```ignore
//! let fixture = ScenarioFixture::builder()
//!     .memory_size(64)
//!     .budget(5)
//!     .max_steps(1_000)
//!     .register(|rt| { rt.register_unit_with(|id| MyUnit::new(id)); })
//!     .build();
//! ```

use crate::world::{
    CountingUnit, DmaSubmitter, MailboxProducer, MailboxResponder, MailboxSender, PollingUnit,
    SignalEmitter, WritingUnit,
};
use std::cell::RefCell;
use std::rc::Rc;

use cellgov_core::{Runtime, SpuFactory};
use cellgov_exec::{FakeIsaUnit, FakeOp};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::state::PpuState;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::codegen::trampoline::encode_li_sc;
use cellgov_ps3_abi::lv2::syscall::PROCESS_EXIT;
use cellgov_time::Budget;

/// One-shot callback populating a fresh runtime with units, mailboxes,
/// and other runtime-owned state.
type RegisterFn = Box<dyn FnOnce(&mut Runtime)>;

/// One-shot callback seeding committed guest memory before runtime
/// construction.
type SeedMemoryFn = Box<dyn FnOnce(&mut GuestMemory)>;

/// Runtime construction inputs plus registration and memory-seed callbacks.
pub struct ScenarioFixture {
    pub(crate) memory_size: usize,
    pub(crate) budget: Budget,
    pub(crate) max_steps: usize,
    pub(crate) seed_memory: SeedMemoryFn,
    pub(crate) register: RegisterFn,
}

impl ScenarioFixture {
    /// Zero-byte memory, zero budget, one max step, no units.
    pub fn empty() -> Self {
        Self {
            memory_size: 0,
            budget: Budget::new(0),
            max_steps: 1,
            seed_memory: Box::new(|_| {}),
            register: Box::new(|_| {}),
        }
    }

    /// Fresh builder with default settings.
    pub fn builder() -> ScenarioFixtureBuilder {
        ScenarioFixtureBuilder::default()
    }

    /// Build a ready-to-step `Runtime`, consuming the fixture.
    ///
    /// The caller is responsible for stepping.
    pub fn build_runtime(self) -> Runtime {
        let mut memory = GuestMemory::new(self.memory_size);
        (self.seed_memory)(&mut memory);
        let mut rt = Runtime::new(memory, self.budget, self.max_steps);
        (self.register)(&mut rt);
        rt
    }
}

/// Builder for [`ScenarioFixture`]. Defaults: 16-byte memory, budget 1,
/// 1000-step cap, no-op callbacks.
pub struct ScenarioFixtureBuilder {
    memory_size: usize,
    budget: Budget,
    max_steps: usize,
    seed_memory: SeedMemoryFn,
    register: RegisterFn,
}

impl Default for ScenarioFixtureBuilder {
    fn default() -> Self {
        Self {
            memory_size: 16,
            budget: Budget::new(1),
            max_steps: 1_000,
            seed_memory: Box::new(|_| {}),
            register: Box::new(|_| {}),
        }
    }
}

impl ScenarioFixtureBuilder {
    /// Committed-memory size in bytes.
    pub fn memory_size(mut self, bytes: usize) -> Self {
        self.memory_size = bytes;
        self
    }

    /// Per-step budget granted to the selected unit.
    pub fn budget(mut self, budget: Budget) -> Self {
        self.budget = budget;
        self
    }

    /// Max-steps cap; the deadlock-detector trip point.
    pub fn max_steps(mut self, steps: usize) -> Self {
        self.max_steps = steps;
        self
    }

    /// Memory-seed callback; runs against a fresh `GuestMemory` before the
    /// runtime is built. Replaces any previous callback.
    pub fn seed_memory<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&mut GuestMemory) + 'static,
    {
        self.seed_memory = Box::new(f);
        self
    }

    /// Registration callback; receives the live runtime once at
    /// construction time. Replaces any previous callback.
    pub fn register<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&mut Runtime) + 'static,
    {
        self.register = Box::new(f);
        self
    }

    /// Finalize into a [`ScenarioFixture`].
    pub fn build(self) -> ScenarioFixture {
        ScenarioFixture {
            memory_size: self.memory_size,
            budget: self.budget,
            max_steps: self.max_steps,
            seed_memory: self.seed_memory,
            register: self.register,
        }
    }
}

/// `unit_count` [`CountingUnit`]s, each finishing after `steps_per_unit`
/// steps, with budget 1 and round-robin scheduling.
pub fn round_robin_fairness_scenario(unit_count: usize, steps_per_unit: u64) -> ScenarioFixture {
    assert!(
        unit_count > 0,
        "round_robin_fairness_scenario needs at least 1 unit"
    );
    assert!(
        steps_per_unit > 0,
        "round_robin_fairness_scenario needs at least 1 step per unit"
    );
    let cap = unit_count
        .checked_mul(steps_per_unit as usize)
        .and_then(|n| n.checked_add(1))
        .expect("round_robin_fairness_scenario step cap overflow");
    ScenarioFixture::builder()
        .memory_size(16)
        .budget(Budget::new(1))
        .max_steps(cap)
        .register(move |rt: &mut Runtime| {
            for _ in 0..unit_count {
                rt.register_unit_with(|id| CountingUnit::new(id, steps_per_unit));
            }
        })
        .build()
}

/// A [`DmaSubmitter`] Put from addr 0 to addr 128 paired with a
/// [`CountingUnit`] that burns ticks until the default
/// `FixedLatency(10)` completion fires.
pub fn dma_block_unblock_scenario() -> ScenarioFixture {
    let src = ByteRange::new(GuestAddr::new(0), 4).unwrap();
    let dst = ByteRange::new(GuestAddr::new(128), 4).unwrap();
    let seed = vec![0xde, 0xad, 0xbe, 0xef];
    ScenarioFixture::builder()
        .memory_size(256)
        .budget(Budget::new(1))
        .max_steps(30)
        .register(move |rt: &mut Runtime| {
            rt.register_unit_with(|id| DmaSubmitter::new(id, src, dst, seed.clone()));
            rt.register_unit_with(|id| CountingUnit::new(id, 20));
        })
        .build()
}

/// Two [`WritingUnit`]s writing into the same 4-byte range, each running
/// `steps_per_unit` steps under round-robin scheduling.
pub fn write_conflict_scenario(steps_per_unit: u64) -> ScenarioFixture {
    assert!(
        steps_per_unit > 0,
        "write_conflict_scenario needs at least 1 step per unit"
    );
    let cap = (2usize)
        .checked_mul(steps_per_unit as usize)
        .and_then(|n| n.checked_add(1))
        .expect("write_conflict_scenario step cap overflow");
    let range = ByteRange::new(GuestAddr::new(0), 4).unwrap();
    ScenarioFixture::builder()
        .memory_size(16)
        .budget(Budget::new(1))
        .max_steps(cap)
        .register(move |rt: &mut Runtime| {
            rt.register_unit_with(|id| WritingUnit::new(id, steps_per_unit, range));
            rt.register_unit_with(|id| WritingUnit::new(id, steps_per_unit, range));
        })
        .build()
}

/// Two [`WritingUnit`]s over one word, each with a fixed value of its
/// own.
///
/// The order decides which value lands last, so a complete schedule
/// reaches one of two final memories. A search that drops an
/// equivalence class reaches fewer. [`write_conflict_scenario`] cannot
/// witness that, because its units write their own step numbers and
/// end on the same byte whatever the order.
///
/// The outcome count holds at two for every `steps_per_unit`. The
/// classes number `C(2n, n)` -- 2, 6, 20 and 70 for one through four
/// steps a unit -- which is what an optimal search costs here.
pub fn store_order_scenario(steps_per_unit: u64) -> ScenarioFixture {
    assert!(
        steps_per_unit > 0,
        "store_order_scenario needs at least 1 step per unit"
    );
    let cap = (2usize)
        .checked_mul(steps_per_unit as usize)
        .and_then(|n| n.checked_add(1))
        .expect("store_order_scenario step cap overflow");
    let range = ByteRange::new(GuestAddr::new(0), 4).unwrap();
    ScenarioFixture::builder()
        .memory_size(16)
        .budget(Budget::new(1))
        .max_steps(cap)
        .register(move |rt: &mut Runtime| {
            rt.register_unit_with(|id| WritingUnit::of_value(id, steps_per_unit, range, 0xA1));
            rt.register_unit_with(|id| WritingUnit::of_value(id, steps_per_unit, range, 0xB2));
        })
        .build()
}

/// A [`PollingUnit`] whose step count the schedule decides, beside the
/// [`WritingUnit`] that releases it.
///
/// The poller finishes once it reads a non-zero byte. So a schedule
/// that runs the writer first costs one poll step, and one that polls
/// first costs `poll_limit` of them. That gap drives a replay onto a
/// cap its own baseline fit inside.
pub fn schedule_dependent_length_scenario(poll_limit: u64) -> ScenarioFixture {
    assert!(
        poll_limit > 1,
        "the poller needs room to outrun a schedule that releases it early"
    );
    let gate = ByteRange::new(GuestAddr::new(8), 1).unwrap();
    let cap = (poll_limit as usize)
        .checked_add(4)
        .expect("schedule_dependent_length_scenario step cap overflow");
    ScenarioFixture::builder()
        .memory_size(16)
        .budget(Budget::new(1))
        .max_steps(cap)
        .register(move |rt: &mut Runtime| {
            rt.register_unit_with(|id| WritingUnit::of_value(id, 1, gate, 0xFF));
            rt.register_unit_with(|id| PollingUnit::new(id, poll_limit, gate));
        })
        .build()
}

/// A single mailbox plus a [`MailboxProducer`] sending `message_count`
/// words `1..=N` into it.
pub fn mailbox_send_scenario(message_count: u64) -> ScenarioFixture {
    assert!(
        message_count > 0,
        "mailbox_send_scenario needs at least 1 message"
    );
    let cap = (message_count as usize)
        .checked_add(1)
        .expect("mailbox_send_scenario step cap overflow");
    ScenarioFixture::builder()
        .memory_size(16)
        .budget(Budget::new(1))
        .max_steps(cap)
        .register(move |rt: &mut Runtime| {
            let target = rt.mailbox_registry_mut().register(4);
            rt.register_unit_with(|id| MailboxProducer::new(id, target, message_count));
        })
        .build()
}

/// [`MailboxSender`] and [`MailboxResponder`] exchanging a command and
/// `command + 1` response through two mailboxes.
pub fn mailbox_roundtrip_scenario(command: u32) -> ScenarioFixture {
    ScenarioFixture::builder()
        .memory_size(16)
        .budget(Budget::new(1))
        .max_steps(20)
        .register(move |rt: &mut Runtime| {
            let cmd_mb = rt.mailbox_registry_mut().register(4);
            let resp_mb = rt.mailbox_registry_mut().register(4);
            // Registration order pins sender to id 0, responder to id 1.
            let sender_id = cellgov_event::UnitId::new(0);
            let responder_id = cellgov_event::UnitId::new(1);
            rt.register_unit_with(|id| {
                MailboxSender::new(id, responder_id, cmd_mb, resp_mb, command)
            });
            rt.register_unit_with(|id| MailboxResponder::new(id, sender_id, cmd_mb, resp_mb));
        })
        .build()
}

/// A single signal register plus a [`SignalEmitter`] OR-ing in the low
/// `bit_count` bits across `bit_count` steps.
///
/// # Panics
///
/// Panics if `bit_count == 0` or `bit_count > 32`.
pub fn signal_update_scenario(bit_count: u64) -> ScenarioFixture {
    assert!(bit_count > 0, "signal_update_scenario needs at least 1 bit");
    assert!(
        bit_count <= 32,
        "signal_update_scenario bit_count must be <= 32, got {bit_count}"
    );
    let cap = (bit_count as usize)
        .checked_add(1)
        .expect("signal_update_scenario step cap overflow");
    ScenarioFixture::builder()
        .memory_size(16)
        .budget(Budget::new(1))
        .max_steps(cap)
        .register(move |rt: &mut Runtime| {
            let target = rt.signal_registry_mut().register();
            rt.register_unit_with(|id| SignalEmitter::new(id, target, bit_count));
        })
        .build()
}

/// A single [`FakeIsaUnit`] running `LoadImm(0xAB)` -> `SharedStore` ->
/// `MailboxSend` -> `End` against mailbox 0.
pub fn fake_isa_scenario() -> ScenarioFixture {
    ScenarioFixture::builder()
        .memory_size(256)
        .budget(Budget::new(1))
        .max_steps(20)
        .register(move |rt: &mut Runtime| {
            rt.mailbox_registry_mut().register(4); // mailbox 0
            rt.register_unit_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xab),
                        FakeOp::SharedStore { addr: 0, len: 4 },
                        FakeOp::MailboxSend { mailbox: 0 },
                        FakeOp::End,
                    ],
                )
            });
        })
        .build()
}

/// `li r11, sys_process_exit; sc`, placed at guest address 0: a
/// microtest's `main` returns to LR 0, so returning exits the process.
fn process_exit_stub() -> [u8; 8] {
    encode_li_sc(PROCESS_EXIT as u16)
}

/// An LV2-driven microtest: a PPU ELF that creates its SPU threads
/// through syscalls.
///
/// The PPU image loads into a 256 MiB-plus-128 KiB memory with its
/// stack pointer 4 KiB below the top and LR 0, where a
/// `li r11, sys_process_exit; sc` stub sits. The fixture registers
/// `spu_elf` under `/app_home/spu_main.elf` for the PPU to load, and
/// `spu_factory` materializes each SPU thread the guest creates.
///
/// A PPU image that does not load registers no unit, so the runtime
/// this fixture builds has an empty registry; the caller checks it. A
/// debug build names the loader's refusal first.
pub fn lv2_driven_scenario(
    ppu_elf: Vec<u8>,
    spu_elf: Vec<u8>,
    budget: Budget,
    max_steps: usize,
    spu_factory: SpuFactory,
) -> ScenarioFixture {
    const MEM_SIZE: usize = 0x1002_0000;
    const STACK_GAP: u64 = 0x1000;
    let primed: Rc<RefCell<Option<PpuState>>> = Rc::new(RefCell::new(None));
    let primed_seed = Rc::clone(&primed);

    ScenarioFixture::builder()
        .memory_size(MEM_SIZE)
        .budget(budget)
        .max_steps(max_steps)
        .seed_memory(move |mem| {
            let stub = process_exit_stub();
            // A refusal here cannot leave the callback, so it names its
            // cause in a debug build; the empty registry it leaves is
            // what a release caller checks.
            let Some(stub_range) = ByteRange::new(GuestAddr::new(0), stub.len() as u64) else {
                debug_assert!(false, "the exit stub's range does not fit guest memory");
                return;
            };
            if let Err(error) = mem.apply_commit(stub_range, &stub) {
                debug_assert!(false, "the exit stub could not be placed: {error}");
                return;
            }
            let mut state = PpuState::new();
            if let Err(error) = cellgov_ppu::loader::load_ppu_elf(&ppu_elf, mem, &mut state) {
                debug_assert!(false, "the microtest PPU ELF failed to load: {error}");
                return;
            }
            state.set_gpr(1, (MEM_SIZE as u64) - STACK_GAP);
            state.set_lr(0);
            *primed_seed.borrow_mut() = Some(state);
        })
        .register(move |rt: &mut Runtime| {
            rt.lv2_host_mut()
                .content_store_mut()
                .register(b"/app_home/spu_main.elf", spu_elf);
            rt.set_spu_factory(spu_factory);
            let Some(ppu_state) = primed.borrow_mut().take() else {
                return;
            };
            rt.register_unit_with(|id| {
                let mut unit = PpuExecutionUnit::new(id);
                *unit.state_mut() = ppu_state;
                unit
            });
        })
        .build()
}

#[cfg(test)]
#[path = "tests/fixtures_tests.rs"]
mod tests;
