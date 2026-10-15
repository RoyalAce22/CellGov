//! The signal-notification values the kernel keeps for an SPU thread,
//! held to the `spu_snr_outside_run` and `spu_snr_run_save` captures
//! from the reference console. Each test makes the probe's calls, in
//! its order, through the runtime's dispatch; each start builds a fresh
//! stand-in SPU, and the test reads the registers the runtime gave it
//! as the probe's SPU program reports them.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, FakeIsaUnit, ProblemStateError,
    SignalNotifier, StopRegisters, UnitStatus,
};
use cellgov_lv2::Lv2Request;
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

use crate::runtime::Runtime;

/// The captured payload of `probe` as big-endian words.
fn console_words(probe: &str) -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
        .join("ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

/// One signal-notification register of the stand-in: its mode, whether
/// it holds a value, and the value.
#[derive(Clone, Copy, Default)]
struct Register {
    logical_or: bool,
    pending: bool,
    value: u32,
}

/// An SPU stand-in that keeps its two signal-notification registers
/// and stops with a thread exit.
#[derive(Clone)]
struct SignalUnit {
    inner: FakeIsaUnit,
    registers: [Register; 2],
}

fn index(register: SignalNotifier) -> usize {
    match register {
        SignalNotifier::One => 0,
        SignalNotifier::Two => 1,
    }
}

impl SignalUnit {
    /// What the probe's SPU program reports: per register, the channel
    /// count and the value it reads (0xFFFFFFFF when the count is 0).
    fn report(&self) -> [u32; 4] {
        let [one, two] = self.registers.map(|r| {
            if r.pending {
                (1, r.value)
            } else {
                (0, 0xFFFF_FFFF)
            }
        });
        [one.0, one.1, two.0, two.1]
    }
}

impl ExecutionUnit for SignalUnit {
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

    fn stop_registers(&self) -> Option<StopRegisters> {
        Some(StopRegisters {
            status: (u32::from(cellgov_ps3_abi::lv2::spu::stop_code::THREAD_EXIT) << 16) | 0b10,
            npc: 0x40,
        })
    }

    fn read_out_mbox(&mut self) -> Result<Option<u32>, ProblemStateError> {
        Ok(Some(0))
    }

    /// [CBEA p:239 s:16.4] each register either overwrites its contents or ORs the data written into them.
    fn write_signal(
        &mut self,
        register: SignalNotifier,
        value: u32,
    ) -> Result<(), ProblemStateError> {
        let r = &mut self.registers[index(register)];
        r.value = if r.logical_or && r.pending {
            r.value | value
        } else {
            value
        };
        r.pending = true;
        Ok(())
    }

    fn set_signal_logical_or(
        &mut self,
        register: SignalNotifier,
        logical_or: bool,
    ) -> Result<(), ProblemStateError> {
        self.registers[index(register)].logical_or = logical_or;
        Ok(())
    }
}

/// A runtime with one one-thread group, its thread initialized, and the
/// PPU that makes the probe's calls.
struct Probe {
    rt: Runtime,
    caller: UnitId,
    group: u32,
    thread: u32,
}

impl Probe {
    fn new() -> Self {
        let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(1), 10);
        let caller = rt.register_unit_with(|id| FakeIsaUnit::new(id, vec![]));
        let handle = rt
            .lv2_host_mut()
            .content_store_mut()
            .register(b"/spu.elf", vec![0x7F, b'E', b'L', b'F']);
        let groups = rt.lv2_host_mut().thread_groups_mut();
        let group = groups.create(1).expect("a group id");
        groups
            .initialize_thread(group, 0, handle, [0; 4])
            .expect("slot 0");
        rt.set_spu_factory(|id, _| {
            Ok(Box::new(SignalUnit {
                inner: FakeIsaUnit::new(id, vec![]),
                registers: [Register::default(); 2],
            }))
        });
        let thread = group * cellgov_lv2::thread_group::MAX_SLOTS_PER_GROUP;
        Self {
            rt,
            caller,
            group,
            thread,
        }
    }

    /// The code the PPU reads back from `request`.
    fn call(&mut self, request: Lv2Request) -> u32 {
        self.rt.dispatch_lv2_request(request, self.caller);
        self.rt
            .registry
            .drain_syscall_return(self.caller)
            .expect("an immediate answer") as u32
    }

    fn set_cfg(&mut self, value: u64) -> u32 {
        self.call(Lv2Request::SpuThreadSetSpuCfg {
            thread_id: self.thread,
            value,
        })
    }

    fn write_snr(&mut self, number: u32, value: u32) -> u32 {
        self.call(Lv2Request::SpuThreadWriteSnr {
            thread_id: self.thread,
            number,
            value,
        })
    }

    /// The mailbox write that releases the probe's SPU.
    fn write_mb(&mut self, value: u32) -> u32 {
        self.call(Lv2Request::SpuThreadWriteMb {
            thread_id: self.thread,
            value,
        })
    }

    /// Start the group; returns its SPU.
    fn start(&mut self) -> UnitId {
        let code = self.call(Lv2Request::SpuThreadGroupStart {
            group_id: self.group,
        });
        assert_eq!(code, 0, "start");
        self.rt
            .lv2_host()
            .thread_groups()
            .running_unit_for_thread(self.thread)
            .expect("the started thread's SPU")
    }

    fn report(&self, spu: UnitId) -> [u32; 4] {
        self.rt
            .registry
            .get(spu)
            .and_then(|u| u.as_any().downcast_ref::<SignalUnit>())
            .expect("a SignalUnit")
            .report()
    }

    /// The SPU exits its thread, which ends the one-thread group.
    fn exit(&mut self, spu: UnitId) {
        let stopped = self.rt.registry.get(spu).and_then(|u| u.stop_registers());
        self.rt.settle_finished_unit(spu, stopped);
    }
}

/// Signal writes before the first start, and one between two runs, are
/// kept and loaded by the next start, in the mode the configuration
/// gives each register.
#[test]
fn signal_writes_outside_a_run_reach_the_next_start_as_on_the_console() {
    let console = console_words("spu_snr_outside_run");
    assert_eq!((console[0], console[1]), (0, 0), "the probe ran every step");

    let mut p = Probe::new();
    assert_eq!(p.set_cfg(1), 0);
    let mut got = vec![0, 0];
    for (number, value) in [(0, 0x10), (0, 0x01), (1, 0x20), (1, 0x40)] {
        got.push(p.write_snr(number, value));
    }
    let spu = p.start();
    got.extend(p.report(spu));
    p.exit(spu);

    got.push(p.write_snr(0, 0x100));
    let spu = p.start();
    got.extend(p.report(spu));
    p.exit(spu);
    got.push(p.call(Lv2Request::SpuThreadGroupDestroy { id: p.group }));

    assert_eq!(got, console[..16], "words 0 to 15");
}

/// A start loads nothing into a register no write reached, and a write
/// during a run reaches that run's SPU but not the next start.
#[test]
fn a_start_loads_no_unwritten_register_and_no_write_from_a_run_as_on_the_console() {
    let console = console_words("spu_snr_run_save");
    assert_eq!((console[0], console[1]), (0, 0), "the probe ran every step");

    let mut p = Probe::new();
    assert_eq!(p.set_cfg(1), 0);
    let mut got = vec![0, 0];
    let spu = p.start();
    got.extend(p.report(spu));
    got.push(p.write_snr(0, 0x2));
    got.push(p.write_mb(1));
    got.extend(p.report(spu));
    p.exit(spu);

    let spu = p.start();
    got.extend(p.report(spu));
    got.push(p.write_mb(1));
    got.extend(p.report(spu));
    p.exit(spu);
    got.push(p.call(Lv2Request::SpuThreadGroupDestroy { id: p.group }));

    assert_eq!(got, console[..22], "words 0 to 21");
}
