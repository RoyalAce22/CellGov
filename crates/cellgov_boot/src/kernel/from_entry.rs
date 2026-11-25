//! A kernel run from its entry under CellGov's hypervisor, with no title,
//! until the first thing CellGov does not model.
//!
//! The run gives the evidence a hypervisor table is built from: the
//! repository reads the kernel makes, in order, and where it stops.

use std::collections::{BTreeSet, VecDeque};

use cellgov_core::{Runtime, RuntimeMode, StepError};
use cellgov_effects::FaultKind;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionStepResult, UnitStatus};
use cellgov_lv1::{
    allocated_storage, page_table_storage, real_mode_size, shadow_storage, HypercallRefusal,
    PausedThread, RepositoryRead,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, MemError, PageSize, Region};
use cellgov_ppu::loader::checked_pt_loads;
use cellgov_ppu::mmu::{Partition, PartitionRefusal};
use cellgov_ppu::supervisor::SupervisorState;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::ppu_interrupt::VECTOR_SYSTEM_RESET_THREAD1;
use cellgov_ps3_abi::hw::ppu_mmu::{rmls_of_limit, REAL_MODE_IGNORED_EA};
use cellgov_ps3_abi::hw::ppu_spr::{
    CTRL_CT_THREAD0, CTRL_CT_THREAD1, CTRL_TE_THREAD0, CTRL_TE_THREAD1,
};
use cellgov_ps3_abi::hw::ppu_supervisor::MSR_SF;
use cellgov_time::Budget;

use super::device_mmio::{declare_device_mmio, device_access, DeviceMmioAccess};
use super::map::{map_kernel, KernelMapError};
use crate::diag::is_hypercall;

/// The budget of one step of either unit.
const STEP_BUDGET: u64 = 256;

/// How many step-end pcs of each hardware thread a run keeps.
pub const RECENT_PCS: usize = 8;

/// The CTRL TE bits of the PPE hardware threads that CellGov enables at
/// the boot: both of them.
///
/// The kernel cannot start or stop a hardware thread: a TE write outside
/// hypervisor state is ignored [CBE-Handbook p:321 s:10.8.1]. So CellGov,
/// as the hypervisor, decides. It enables thread 1 with thread 0, and
/// thread 1 waits in the kernel's claim-word spin until thread 0 releases
/// it. The `lv2_record_port_ids` console capture shows thread 1 run
/// kernel code during the boot.
const ENABLED_THREADS: u32 = CTRL_TE_THREAD0 | CTRL_TE_THREAD1;

/// The CTRL value that hardware thread `thread` reads while its sibling
/// runs: its own thread in CT and [`ENABLED_THREADS`] in TE.
///
/// [CBE-Handbook p:307 s:10.4.4] CT names the thread that reads CTRL, and TE holds the enabled threads.
fn ctrl_of(thread: u8) -> u32 {
    let current = match thread {
        0 => CTRL_CT_THREAD0,
        _ => CTRL_CT_THREAD1,
    };
    current | ENABLED_THREADS
}

/// Why a kernel run from its entry stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelRunStop {
    /// CellGov's hypervisor refused a hypercall.
    Refused(HypercallRefusal),
    /// A hardware thread faulted.
    Fault {
        /// The PPE hardware thread that faulted.
        thread: u8,
        /// The fault.
        fault: FaultKind,
        /// The address of the faulting instruction, when the unit named it.
        pc: Option<u64>,
    },
    /// A hardware thread accessed the MMIO of a device. The console
    /// performs the access, and CellGov does not model the device.
    DeviceMmio(DeviceMmioAccess),
    /// lv1 paused every hardware thread, and no wake came.
    ///
    /// On the console, one of these wakes a paused thread:
    /// - an interrupt on an unmasked plug, which the runtime delivers: the
    ///   thread runs again and returns from `pause`;
    /// - in mode 0, the thread's decrementer, which CellGov does not model.
    ///
    /// With both threads paused, no thread runs to raise a plug, so the run
    /// cannot continue.
    NoWake {
        /// The paused threads, in thread order.
        paused: Vec<PausedThread>,
    },
    /// The run reached its step cap.
    StepCap,
    /// The runtime refused a step for another reason.
    Step(StepError),
    /// The runtime refused to commit a step, for the reason given.
    Commit(String),
}

/// What a kernel run from its entry produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelRun {
    /// The steps the runtime took, both units together.
    pub steps: usize,
    /// Why the run stopped.
    pub stop: KernelRunStop,
    /// Every repository read the kernel made, in order.
    pub repository_reads: Vec<RepositoryRead>,
    /// Every byte the kernel wrote to the logical console.
    pub console: Vec<u8>,
    /// Where hardware thread 0 is when the run stops. It is `None` when
    /// the runtime holds no PPU unit with supervisor registers under its id.
    pub thread0: Option<KernelPosition>,
    /// Where hardware thread 1 is when the run stops, as for
    /// [`Self::thread0`].
    pub thread1: Option<KernelPosition>,
    /// Each distinct hypercall site of each hardware thread, by thread
    /// number. A run keeps each pair of pc and number once, and keeps
    /// the call that a refusal stops.
    pub hypercall_sites: [BTreeSet<KernelHypercallSite>; 2],
}

/// One hardware thread's unit when a run from its entry stops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelPosition {
    /// The address of the next instruction that the unit runs, or of the
    /// `sc` of a hypercall before its answer reaches the unit. The PPU
    /// keeps the `sc` as the pc until the answer moves it past the `sc`
    /// (cellgov_ppu `unit/hypercall.rs` `hypercall_yield`). So a run that
    /// a refused hypercall stops leaves the pc on that `sc`, and so does a
    /// pause that no wake ends.
    pub pc: u64,
    /// The unit's machine state register.
    pub msr: u64,
    /// The unit's scheduling status, or `None` when the runtime gives none.
    pub status: Option<UnitStatus>,
    /// The unit's last hypercall, or `None` when it made none.
    pub last_hypercall: Option<KernelHypercall>,
    /// The unit's pc at the end of each of its last committed steps that
    /// did not fault, oldest first. The run keeps at most [`RECENT_PCS`]
    /// of them. A step that faults is not among them: the PPU discards
    /// its batch and the pc stays where the step began (cellgov_ppu
    /// `unit/fault.rs` `fault_yield`). A step whose commit the runtime
    /// refuses is not among them either.
    pub recent_pcs: Vec<u64>,
}

/// One hypercall of a hardware thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KernelHypercall {
    /// The hypercall number from r11.
    pub number: u64,
    /// The address of the `sc` that made the call.
    pub pc: u64,
    /// The runtime step that made the call, counted from 1 over both units.
    pub step: usize,
}

/// One hypercall number that a hardware thread gave at one `sc`.
///
/// The order is by pc, then by number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct KernelHypercallSite {
    /// The address of the `sc`, as the unit's pc gives it.
    pub pc: u64,
    /// The hypercall number from r11.
    pub number: u64,
}

/// What a run records about one hardware thread's unit as it steps.
#[derive(Debug, Default)]
struct KernelTrail {
    last_hypercall: Option<KernelHypercall>,
    hypercall_sites: BTreeSet<KernelHypercallSite>,
    recent_pcs: VecDeque<u64>,
}

impl KernelTrail {
    /// Keeps `call` as the last hypercall and as one of the sites.
    fn push_hypercall(&mut self, call: KernelHypercall) {
        self.hypercall_sites.insert(KernelHypercallSite {
            pc: call.pc,
            number: call.number,
        });
        self.last_hypercall = Some(call);
    }

    /// Keeps `pc` as the newest step-end pc and drops the oldest one past
    /// [`RECENT_PCS`].
    fn push_pc(&mut self, pc: u64) {
        if self.recent_pcs.len() == RECENT_PCS {
            self.recent_pcs.pop_front();
        }
        self.recent_pcs.push_back(pc);
    }
}

/// The units of the two hardware threads, by thread.
#[derive(Debug, Clone, Copy)]
struct Threads([UnitId; 2]);

impl Threads {
    /// The hardware thread whose unit is `unit`, or `None` for another
    /// unit.
    fn thread_of(&self, unit: UnitId) -> Option<u8> {
        (0u8..)
            .zip(self.0)
            .find(|&(_, id)| id == unit)
            .map(|(t, _)| t)
    }
}

/// Why a kernel run from its entry did not start.
#[derive(Debug, thiserror::Error)]
pub enum KernelRunError {
    /// The ELF does not map at its link address.
    #[error(transparent)]
    Map(#[from] KernelMapError),
    /// The real-mode address space refused the region or a segment.
    #[error("real-mode kernel address space: {source}")]
    RealMode {
        /// The address space's own account of the refusal.
        source: MemError,
    },
    /// A segment's end passes the top of the address space.
    #[error("kernel segment at real address 0x{at:016x} with 0x{len:x} bytes overflows")]
    Overflow {
        /// The segment's real address.
        at: u64,
        /// Its file size.
        len: u64,
    },
    /// A segment, its zero fill included, reaches past the region.
    #[error(
        "kernel segment at real address 0x{at:016x} with 0x{len:x} bytes reaches past \
         real address 0x{end:x}, where the real-mode kernel region ends"
    )]
    OutsideRegion {
        /// The segment's real address.
        at: u64,
        /// Its memory size.
        len: u64,
        /// The real address where the region ends: the partition's real
        /// mode size.
        end: u64,
    },
    /// No repository row gives the partition's real mode size at the
    /// firmware.
    #[error("no repository row gives the real mode size at firmware {firmware}")]
    NoRealModeSize {
        /// The firmware version.
        firmware: String,
    },
    /// The partition's real mode size is a limit that no LPCR\[RMLS\]
    /// value selects.
    #[error("real mode size 0x{size:x} at firmware {firmware} is a limit no RMLS value selects")]
    RealModeSize {
        /// The firmware version.
        firmware: String,
        /// The size that the repository row gives.
        size: u64,
    },
    /// The PPU refused the partition's resources.
    #[error("kernel partition: {source}")]
    Partition {
        /// The PPU's account of the refusal.
        source: PartitionRefusal,
    },
}

/// Runs the kernel ELF `elf` of the firmware version `firmware` from its
/// entry, on both PPE hardware threads, for at most `step_cap` steps.
///
/// Each kernel segment lies at its real address. Real mode ignores the
/// two high effective-address bits, so the link address loses its top
/// bit. CellGov's hypervisor answers the kernel's repository reads with
/// the values for `firmware`. [`KernelRunStop`] gives the reason the run
/// stops.
///
/// Each thread starts in supervisor state with 64-bit mode on, MSR\[SF\],
/// and with every other register at 0:
/// - thread 0 at the ELF's entry;
/// - thread 1 at its system reset vector, [`VECTOR_SYSTEM_RESET_THREAD1`].
///
/// The MSR of thread 0 is the start MSR that lv1 gives the logical PPE
/// of a partition [OFW-4.93 lv1 a:0x2E0EA0].
///
/// Both threads hold the partition that [`kernel_partition`] gives for
/// `firmware`. So a real-mode access at or past the partition's real
/// mode size takes the storage interrupt. The region at real address 0
/// ends at that size, so a real-mode access below it reads memory.
///
/// Each thread reads its own thread in CTRL CT, and both enabled threads
/// in TE. While lv1 holds the other thread paused, TE holds only the
/// reader's own thread.
///
/// # Errors
///
/// Returns a [`KernelRunError`] in these cases:
/// - the ELF does not map into the real-mode address space;
/// - no partition comes from the repository rows of `firmware`.
pub fn run_kernel_from_entry(
    elf: &[u8],
    firmware: &str,
    step_cap: usize,
) -> Result<KernelRun, KernelRunError> {
    let entry = map_kernel(elf)?.entry;
    let partition = kernel_partition(firmware)?;
    let mem = real_mode_image(elf, partition.rm_limit(), firmware)?;
    let mut rt = Runtime::new(mem, Budget::new(STEP_BUDGET), step_cap);
    rt.set_mode(RuntimeMode::FaultDriven);
    rt.set_hypervisor_firmware(firmware);
    let threads = Threads([
        register_thread(&mut rt, 0, entry, partition),
        register_thread(&mut rt, 1, VECTOR_SYSTEM_RESET_THREAD1, partition),
    ]);
    let mut trails = [KernelTrail::default(), KernelTrail::default()];
    let stop = drive(&mut rt, firmware, threads, &mut trails);
    let hypercall_sites = trails
        .each_mut()
        .map(|t| std::mem::take(&mut t.hypercall_sites));
    let [trail0, trail1] = trails;
    Ok(KernelRun {
        steps: rt.steps_taken(),
        stop,
        repository_reads: rt.hypervisor().repository_reads().to_vec(),
        console: rt.hypervisor().console_text().to_vec(),
        thread0: position(&rt, threads.0[0], trail0),
        thread1: position(&rt, threads.0[1], trail1),
        hypercall_sites,
    })
}

/// The partition that the kernel's threads hold at entry at `firmware`.
///
/// - The real mode limit is `bi.pu.1.rm_size` from the repository rows.
/// - The real mode offset is 0, the partition address of the region,
///   `bi.pu.1.rm_addr`. A CellGov real address is a partition address.
/// - SDR1 and HID6\[LB\] are those of [`Partition::INITIAL`]. Until the
///   kernel selects an address space, lv1's page table for the PPE is in
///   lv1's own memory, at an address that no committed source gives. No
///   access reads SDR1 in that time, because the threads start with
///   translation off.
///
/// lv1 loads these resources when it switches a thread to the PPE:
/// - LPCR from the thread's lv1 context and RMOR from the PPE object
///   [OFW-4.93 lv1 a:0x2C47A0-0x2C47AC];
/// - SDR1 from the page table of the PPE's address space
///   [OFW-4.93 lv1 a:0x2C47BC-0x2C47D0].
///
/// [CBE-Handbook p:336 s:11.2] LPCR\[RMLS\] selects the real mode limit, from the sizes of Table 11-1.
///
/// # Errors
///
/// Returns a [`KernelRunError`] in these cases:
/// - no repository row gives the real mode size at `firmware`;
/// - no RMLS value selects that size;
/// - the PPU refuses the resources.
pub fn kernel_partition(firmware: &str) -> Result<Partition, KernelRunError> {
    let size = real_mode_size(firmware).ok_or_else(|| KernelRunError::NoRealModeSize {
        firmware: firmware.to_owned(),
    })?;
    real_mode_partition(firmware, size)
}

/// The partition of [`kernel_partition`] with the real mode size `size`
/// at `firmware`.
pub(super) fn real_mode_partition(firmware: &str, size: u64) -> Result<Partition, KernelRunError> {
    if rmls_of_limit(size).is_none() {
        return Err(KernelRunError::RealModeSize {
            firmware: firmware.to_owned(),
            size,
        });
    }
    let initial = Partition::INITIAL;
    Partition::new(initial.sdr1(), size, 0, initial.lb())
        .map_err(|source| KernelRunError::Partition { source })
}

/// Registers the unit of hardware thread `thread` in `rt`, at `pc`, with
/// the resources of `partition`.
fn register_thread(rt: &mut Runtime, thread: u8, pc: u64, partition: Partition) -> UnitId {
    rt.register_unit_with(|id| {
        let mut unit = PpuExecutionUnit::new(id);
        let mut supervisor = SupervisorState::new(MSR_SF);
        supervisor.partition = partition;
        unit.state_mut().set_supervisor(Some(supervisor));
        unit.state_mut().pc = pc;
        unit.fix_ctrl(ctrl_of(thread));
        unit
    })
}

/// The unit `unit` of `rt`, with what `trail` recorded about it.
fn position(rt: &Runtime, unit: UnitId, trail: KernelTrail) -> Option<KernelPosition> {
    let state = ppu_unit(rt, unit)?.state();
    Some(KernelPosition {
        pc: state.pc,
        msr: state.supervisor()?.msr,
        status: rt.registry().effective_status(unit),
        last_hypercall: trail.last_hypercall,
        recent_pcs: trail.recent_pcs.into(),
    })
}

/// The PPU unit `id` of `rt`, or `None` when `id` is not a PPU unit.
fn ppu_unit(rt: &Runtime, id: UnitId) -> Option<&PpuExecutionUnit> {
    rt.registry()
        .get(id)?
        .as_any()
        .downcast_ref::<PpuExecutionUnit>()
}

/// Steps `rt` until it stops, and says why. Each step of the unit of
/// hardware thread `t` goes into `trails[t]`. A fault at the MMIO of a
/// device at `firmware` stops the run as [`KernelRunStop::DeviceMmio`].
///
/// The loop has no bound of its own: the runtime's step cap ends it.
fn drive(
    rt: &mut Runtime,
    firmware: &str,
    threads: Threads,
    trails: &mut [KernelTrail; 2],
) -> KernelRunStop {
    loop {
        let mut step = match rt.step() {
            Ok(step) => step,
            Err(error) => return step_refusal(rt, error),
        };
        let thread = threads.thread_of(step.unit);
        let trail = thread.and_then(|t| trails.get_mut(usize::from(t)));
        if let (Some(trail), Some(call)) = (trail, hypercall_of(&step.result, rt.steps_taken())) {
            trail.push_hypercall(call);
        }
        let pc = step.result.local_diagnostics.pc;
        let fault = step.result.fault.zip(thread).map(|(fault, thread)| {
            match device_access(firmware, thread, fault, &step.result) {
                Some(access) => KernelRunStop::DeviceMmio(access),
                None => KernelRunStop::Fault { thread, fault, pc },
            }
        });
        if let Err(error) = rt.commit_step(&mut step) {
            return KernelRunStop::Commit(error.to_string());
        }
        if let Some(stop) = fault {
            return stop;
        }
        let trail = thread.and_then(|t| trails.get_mut(usize::from(t)));
        if let (Some(trail), Some(unit)) = (trail, ppu_unit(rt, step.unit)) {
            trail.push_pc(unit.state().pc);
        }
    }
}

/// The stop for the step refusal `error` of `rt`.
///
/// A run whose units are all parked, with a thread that lv1 paused,
/// stops at [`KernelRunStop::NoWake`].
fn step_refusal(rt: &Runtime, error: StepError) -> KernelRunStop {
    match error {
        StepError::HypercallRefused => match rt.hypervisor().refusal() {
            Some(refusal) => KernelRunStop::Refused(refusal),
            None => KernelRunStop::Step(StepError::HypercallRefused),
        },
        StepError::MaxStepsExceeded => KernelRunStop::StepCap,
        StepError::AllBlocked => {
            let paused: Vec<PausedThread> = rt
                .hypervisor()
                .groups()
                .interrupt()
                .paused()
                .threads()
                .collect();
            if paused.is_empty() {
                KernelRunStop::Step(StepError::AllBlocked)
            } else {
                KernelRunStop::NoWake { paused }
            }
        }
        error => KernelRunStop::Step(error),
    }
}

/// The hypercall that `result` yielded in the runtime step `step`, or
/// `None` when it yielded no hypercall.
fn hypercall_of(result: &ExecutionStepResult, step: usize) -> Option<KernelHypercall> {
    if !is_hypercall(result) {
        return None;
    }
    Some(KernelHypercall {
        number: result.syscall_args?[0],
        pc: result.local_diagnostics.pc?,
        step,
    })
}

/// The real address space of a kernel run:
/// - one region at real address 0, `rm_size` bytes long, that holds each
///   segment of the kernel ELF `elf` at its real address;
/// - the storage of CellGov's hypervisor for its page table;
/// - the storage of CellGov's hypervisor for the shadow pages of the
///   logical SPEs;
/// - the storage of CellGov's hypervisor for the pages of the regions
///   that `allocate_memory` creates;
/// - an unmodelled span over each MMIO region of the devices at
///   `firmware`.
///
/// `rm_size` is the partition's real mode size, so each real-mode access
/// that the partition admits finds memory.
///
/// The caller has mapped `elf` with `map_kernel`, which checks the
/// segments and bounds their span by
/// [`KERNEL_SPAN_LIMIT`](super::map::KERNEL_SPAN_LIMIT). That span
/// starts at the lowest segment, so this function checks that each
/// segment ends inside the region.
fn real_mode_image(
    elf: &[u8],
    rm_size: u64,
    firmware: &str,
) -> Result<GuestMemory, KernelRunError> {
    let segments = checked_pt_loads(elf).map_err(|source| KernelMapError::Elf { source })?;
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, rm_size as usize, "lv2 real", PageSize::Page64K),
        page_table_storage(),
        shadow_storage(),
        allocated_storage(),
    ])
    .map_err(|source| KernelRunError::RealMode { source })?;
    declare_device_mmio(&mut mem, firmware)
        .map_err(|source| KernelRunError::RealMode { source })?;
    for s in segments.iter().filter(|s| s.memsz != 0) {
        // [PPC-Book3 p:29 s:4.2.6] Real addressing ignores EA bit 1, and EA bit 0 picks the mechanism that forms the real address.
        let real = s.vaddr & !REAL_MODE_IGNORED_EA;
        // A segment past the region would lose its zero fill unreported.
        if real.checked_add(s.memsz).is_none_or(|end| end > rm_size) {
            return Err(KernelRunError::OutsideRegion {
                at: real,
                len: s.memsz,
                end: rm_size,
            });
        }
        if s.filesz > 0 {
            let from = s.file_offset as usize;
            commit(&mut mem, real, &elf[from..from + s.filesz as usize])?;
        }
        if s.executable {
            mem.declare_code(real, s.memsz);
        }
    }
    Ok(mem)
}

/// Writes `bytes` at real address `at`.
fn commit(mem: &mut GuestMemory, at: u64, bytes: &[u8]) -> Result<(), KernelRunError> {
    let len = bytes.len() as u64;
    let range =
        ByteRange::new(GuestAddr::new(at), len).ok_or(KernelRunError::Overflow { at, len })?;
    mem.apply_commit(range, bytes)
        .map_err(|source| KernelRunError::RealMode { source })
}
