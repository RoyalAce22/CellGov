//! A kernel run from its entry under CellGov's hypervisor, with no title,
//! until the first thing CellGov does not model.
//!
//! The run gives the evidence a hypervisor table is built from: the
//! repository reads the kernel makes, in order, and where it stops.

use cellgov_core::{Runtime, RuntimeMode, StepError};
use cellgov_effects::FaultKind;
use cellgov_event::UnitId;
use cellgov_lv1::{page_table_storage, HypercallRefusal, RepositoryRead};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, MemError, PageSize, Region};
use cellgov_ppu::loader::checked_pt_loads;
use cellgov_ppu::supervisor::SupervisorState;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::ppc_isa::PPC_B_OPCODE_NO_LK;
use cellgov_ps3_abi::hw::ppu_mmu::REAL_MODE_IGNORED_EA;
use cellgov_ps3_abi::hw::ppu_supervisor::MSR_SF;
use cellgov_time::Budget;

use super::map::{map_kernel, KernelMapError, KERNEL_SPAN_LIMIT};

/// The budget of one step of either unit.
const STEP_BUDGET: u64 = 256;

/// The real address of the placeholder sibling's `b .`: the region's
/// last word.
const SPIN_WORD: u64 = KERNEL_SPAN_LIMIT - 4;

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
/// the values for `firmware`. The run stops at the first hypercall
/// refusal, kernel fault or step refusal.
///
/// # Errors
///
/// Returns a [`KernelRunError`] when the ELF does not map.
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
    let stop = drive(&mut rt, kernel);
    Ok(KernelRun {
        steps: rt.steps_taken(),
        stop,
        repository_reads: rt.hypervisor().repository_reads().to_vec(),
        console: rt.hypervisor().console_text().to_vec(),
    })
}

/// Steps `rt` until it stops, and says why.
fn drive(rt: &mut Runtime, kernel: UnitId) -> KernelRunStop {
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
    }
}

/// The real address space of a kernel run:
/// - one region at real address 0, [`KERNEL_SPAN_LIMIT`] bytes long,
///   that holds each segment of the kernel ELF `elf` at its real
///   address;
/// - the storage of CellGov's hypervisor for its page table.
///
/// The caller has mapped `elf` with `map_kernel`, which checks the
/// segments and bounds their span by [`KERNEL_SPAN_LIMIT`]. That span
/// starts at the lowest segment, so each segment is checked here to end
/// at or below [`SPIN_WORD`].
fn real_mode_image(elf: &[u8]) -> Result<GuestMemory, KernelRunError> {
    let segments = checked_pt_loads(elf).map_err(|source| KernelMapError::Elf { source })?;
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, KERNEL_SPAN_LIMIT as usize, "lv2 real", PageSize::Page64K),
        page_table_storage(),
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
