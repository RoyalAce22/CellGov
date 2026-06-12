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
//!     .register(|r| { r.register_with(|id| MyUnit::new(id)); })
//!     .build();
//! ```

use crate::world::{
    CountingUnit, DmaSubmitter, MailboxProducer, MailboxResponder, MailboxSender, SignalEmitter,
    WritingUnit,
};
use cellgov_core::Runtime;
use cellgov_exec::{FakeIsaUnit, FakeOp};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
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
                rt.registry_mut()
                    .register_with(|id| CountingUnit::new(id, steps_per_unit));
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
            rt.registry_mut()
                .register_with(|id| DmaSubmitter::new(id, src, dst, seed.clone()));
            rt.registry_mut()
                .register_with(|id| CountingUnit::new(id, 20));
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
            rt.registry_mut()
                .register_with(|id| WritingUnit::new(id, steps_per_unit, range));
            rt.registry_mut()
                .register_with(|id| WritingUnit::new(id, steps_per_unit, range));
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
            rt.registry_mut()
                .register_with(|id| MailboxProducer::new(id, target, message_count));
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
            rt.registry_mut()
                .register_with(|id| MailboxSender::new(id, responder_id, cmd_mb, resp_mb, command));
            rt.registry_mut()
                .register_with(|id| MailboxResponder::new(id, sender_id, cmd_mb, resp_mb));
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
            rt.registry_mut()
                .register_with(|id| SignalEmitter::new(id, target, bit_count));
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
            rt.registry_mut().register_with(|id| {
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

#[cfg(test)]
#[path = "tests/fixtures_tests.rs"]
mod tests;
