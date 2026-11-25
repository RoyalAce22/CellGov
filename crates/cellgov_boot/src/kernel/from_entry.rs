//! A kernel run from its entry under CellGov's hypervisor, with no title,
//! until the first thing CellGov does not model.
//!
//! The run gives the evidence a hypervisor table is built from: the
//! repository reads the kernel makes, in order, and where it stops.

use std::collections::VecDeque;

use cellgov_core::{Runtime, RuntimeMode, StepError};
use cellgov_effects::FaultKind;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionStepResult, UnitStatus};
use cellgov_lv1::{
    allocated_storage, page_table_storage, shadow_storage, HypercallRefusal, RepositoryRead,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, MemError, PageSize, Region};
use cellgov_ppu::loader::checked_pt_loads;
use cellgov_ppu::supervisor::SupervisorState;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::ppc_isa::PPC_B_OPCODE_NO_LK;
use cellgov_ps3_abi::hw::ppu_mmu::REAL_MODE_IGNORED_EA;
use cellgov_ps3_abi::hw::ppu_supervisor::MSR_SF;
use cellgov_time::Budget;

use super::map::{map_kernel, KernelMapError, KERNEL_SPAN_LIMIT};
use crate::diag::is_hypercall;

/// The budget of one step of either unit.
const STEP_BUDGET: u64 = 256;

/// The real address of the placeholder sibling's `b .`: the region's
/// last word.
const SPIN_WORD: u64 = KERNEL_SPAN_LIMIT - 4;

/// How many step-end pcs of the kernel's unit a run keeps.
pub const RECENT_PCS: usize = 8;

/// Why a kernel run from its entry stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelRunStop {
    /// CellGov's hypervisor refused a hypercall.
    Refused(HypercallRefusal),
    /// The kernel's unit faulted.
    Fault {
        /// The fault.
        fault: FaultKind,
        /// The address of the faulting instruction, when the unit named it.
        pc: Option<u64>,
    },
    /// The placeholder sibling faulted, most often because the kernel
    /// wrote over its `b .`. The run's thread placement no longer holds.
    SiblingFault {
        /// The fault.
        fault: FaultKind,
        /// The address of the faulting instruction, when the unit named it.
        pc: Option<u64>,
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
    /// Where the kernel's unit is when the run stops. `None` when the
    /// runtime holds no PPU unit with supervisor registers under the
    /// kernel's id.
    pub kernel: Option<KernelPosition>,
}

/// The kernel's unit when a run from its entry stops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelPosition {
    /// The address of the next instruction that the unit runs, or of the
    /// `sc` of a hypercall whose answer has not reached the unit. The PPU
    /// keeps the `sc` as the pc until the answer moves it past
    /// (cellgov_ppu `unit/hypercall.rs` `hypercall_yield`), so a run that
    /// a refused hypercall stops leaves the pc on that `sc`.
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

/// One hypercall of the kernel's unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KernelHypercall {
    /// The hypercall number from r11.
    pub number: u64,
    /// The address of the `sc` that made the call.
    pub pc: u64,
    /// The runtime step that made the call, counted from 1 over both units.
    pub step: usize,
}

/// What a run records about the kernel's unit as it steps.
#[derive(Debug, Default)]
struct KernelTrail {
    last_hypercall: Option<KernelHypercall>,
    recent_pcs: VecDeque<u64>,
}

impl KernelTrail {
    /// Keeps `pc` as the newest step-end pc and drops the oldest one past
    /// [`RECENT_PCS`].
    fn push_pc(&mut self, pc: u64) {
        if self.recent_pcs.len() == RECENT_PCS {
            self.recent_pcs.pop_front();
        }
        self.recent_pcs.push_back(pc);
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
    /// A segment, its zero fill included, reaches past the region or
    /// onto the placeholder sibling's word.
    #[error(
        "kernel segment at real address 0x{at:016x} with 0x{len:x} bytes reaches past \
         real address 0x{SPIN_WORD:x}, where the real-mode kernel region ends"
    )]
    OutsideRegion {
        /// The segment's real address.
        at: u64,
        /// Its memory size.
        len: u64,
    },
}

/// Runs the kernel ELF `elf` of the firmware version `firmware` from its
/// entry, in supervisor state with 64-bit mode on, for at most
/// `step_cap` steps.
///
/// Each kernel segment lies at its real address. Real mode ignores the
/// two high effective-address bits, so the link address loses its top
/// bit. CellGov's hypervisor answers the kernel's repository reads with
/// the values for `firmware`. [`KernelRunStop`] gives the reason the run
/// stops.
///
/// # Errors
///
/// Returns a [`KernelRunError`] when the ELF does not map into the
/// real-mode address space.
pub fn run_kernel_from_entry(
    elf: &[u8],
    firmware: &str,
    step_cap: usize,
) -> Result<KernelRun, KernelRunError> {
    let entry = map_kernel(elf)?.entry;
    let mut mem = real_mode_image(elf)?;
    // Stand-in until the runtime places the boot thread on hardware thread
    // 0 itself: CellGov places a lone PPE unit on thread 1, and the
    // kernel's boot path reads CTRL and runs only on thread 0. A runnable
    // older sibling that branches to itself moves the kernel's unit to
    // thread 0. The sibling's `b .` sits in the region's last word.
    // The `b` template with a zero displacement is `b .`.
    let spin = SPIN_WORD;
    commit(&mut mem, spin, &PPC_B_OPCODE_NO_LK.to_be_bytes())?;
    let mut rt = Runtime::new(mem, Budget::new(STEP_BUDGET), step_cap);
    rt.set_mode(RuntimeMode::FaultDriven);
    rt.set_hypervisor_firmware(firmware);
    rt.register_unit_with(|id| {
        let mut unit = PpuExecutionUnit::new(id);
        unit.state_mut().pc = spin;
        unit
    });
    let kernel = rt.register_unit_with(|id| {
        let mut unit = PpuExecutionUnit::new(id);
        unit.state_mut()
            .set_supervisor(Some(SupervisorState::new(MSR_SF)));
        unit.state_mut().pc = entry;
        unit
    });
    let mut trail = KernelTrail::default();
    let stop = drive(&mut rt, kernel, &mut trail);
    Ok(KernelRun {
        steps: rt.steps_taken(),
        stop,
        repository_reads: rt.hypervisor().repository_reads().to_vec(),
        console: rt.hypervisor().console_text().to_vec(),
        kernel: position(&rt, kernel, trail),
    })
}

/// The unit `kernel` of `rt`, with what `trail` recorded about it.
fn position(rt: &Runtime, kernel: UnitId, trail: KernelTrail) -> Option<KernelPosition> {
    let state = ppu_unit(rt, kernel)?.state();
    Some(KernelPosition {
        pc: state.pc,
        msr: state.supervisor()?.msr,
        status: rt.registry().effective_status(kernel),
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

/// Steps `rt` until it stops, and says why. Each step of the unit
/// `kernel` goes into `trail`.
///
/// The loop has no bound of its own: the runtime's step cap ends it.
fn drive(rt: &mut Runtime, kernel: UnitId, trail: &mut KernelTrail) -> KernelRunStop {
    loop {
        let mut step = match rt.step() {
            Ok(step) => step,
            Err(StepError::HypercallRefused) => {
                return match rt.hypervisor().refusal() {
                    Some(refusal) => KernelRunStop::Refused(refusal),
                    None => KernelRunStop::Step(StepError::HypercallRefused),
                }
            }
            Err(StepError::MaxStepsExceeded) => return KernelRunStop::StepCap,
            Err(error) => return KernelRunStop::Step(error),
        };
        let own = step.unit == kernel;
        if own {
            trail.last_hypercall =
                hypercall_of(&step.result, rt.steps_taken()).or(trail.last_hypercall);
        }
        // A sibling fault stops the run too: the sibling holds the kernel's
        // unit on thread 0, and a run without it goes on a broken setup.
        let pc = step.result.local_diagnostics.pc;
        let fault = step.result.fault.map(|fault| {
            if step.unit == kernel {
                KernelRunStop::Fault { fault, pc }
            } else {
                KernelRunStop::SiblingFault { fault, pc }
            }
        });
        if let Err(error) = rt.commit_step(&mut step) {
            return KernelRunStop::Commit(error.to_string());
        }
        if let Some(stop) = fault {
            return stop;
        }
        if let (true, Some(unit)) = (own, ppu_unit(rt, kernel)) {
            trail.push_pc(unit.state().pc);
        }
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
/// - one region at real address 0, [`KERNEL_SPAN_LIMIT`] bytes long,
///   that holds each segment of the kernel ELF `elf` at its real
///   address;
/// - the storage of CellGov's hypervisor for its page table;
/// - the storage of CellGov's hypervisor for the shadow pages of the
///   logical SPEs;
/// - the storage of CellGov's hypervisor for the pages of the regions
///   that `allocate_memory` creates.
///
/// The caller has mapped `elf` with `map_kernel`, which checks the
/// segments and bounds their span by [`KERNEL_SPAN_LIMIT`]. That span
/// starts at the lowest segment, so this function checks that each
/// segment ends at or below [`SPIN_WORD`].
fn real_mode_image(elf: &[u8]) -> Result<GuestMemory, KernelRunError> {
    let segments = checked_pt_loads(elf).map_err(|source| KernelMapError::Elf { source })?;
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, KERNEL_SPAN_LIMIT as usize, "lv2 real", PageSize::Page64K),
        page_table_storage(),
        shadow_storage(),
        allocated_storage(),
    ])
    .map_err(|source| KernelRunError::RealMode { source })?;
    for s in segments.iter().filter(|s| s.memsz != 0) {
        // [PPC-Book3 p:29 s:4.2.6] Real addressing ignores EA bit 1, and EA bit 0 picks the mechanism that forms the real address.
        let real = s.vaddr & !REAL_MODE_IGNORED_EA;
        // A segment past the region would lose its zero fill unreported,
        // and one over the sibling's word would lose that word to `b .`.
        if real.checked_add(s.memsz).is_none_or(|end| end > SPIN_WORD) {
            return Err(KernelRunError::OutsideRegion {
                at: real,
                len: s.memsz,
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
