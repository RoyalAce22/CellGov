//! The process_spawn_wait microtest: the parent spawns a child SELF,
//! both run under the deterministic scheduler in disjoint address
//! spaces, the parent observes the child's exit, and the whole run
//! is bit-identical across two boots.
//!
//! Skips when the microtest ELFs are absent (they are gitignored;
//! build with tests/micro/process_spawn_wait/build.sh in any
//! ps3dev+PSL1GHT toolchain container -- requirements are in that
//! script's header).

use cellgov_core::{AddressSpaceId, Runtime, SpawnedProcessImage, StepError};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::Budget;

const PARENT_ELF: &str = "../../tests/micro/process_spawn_wait/build/parent.elf";
const CHILD_ELF: &str = "../../tests/micro/process_spawn_wait/build/child.elf";
const CHILD_SELF: &str = "../../tests/micro/process_spawn_wait/build/child.self";

// Covers the ELF's user region plus the SYS_PROCESS_PARAM segment
// at 0x1000_0000 (same sizing as cellgov_ppu's microtest harness).
const MEM_SIZE: usize = 0x1002_0000;
const RESULT_ADDR: u64 = 0x100;
const EXPECTED_PID: u32 = cellgov_ps3_abi::sys_process::BOOT_PROCESS_PID + 0x100;
const CHILD_MAGIC: u32 = 0x600D_F00D;

/// `li r11, 22; sc` -- a terminal `blr` lands on sys_process_exit.
const EXIT_STUB: [u8; 8] = [0x39, 0x60, 0x00, 0x16, 0x44, 0x00, 0x00, 0x02];

/// Initial SP for both spaces; the stub must stay below it or the
/// guest stack would grow down onto it.
const STACK_TOP: u64 = (MEM_SIZE as u64) - 0x1000;

/// Mirrors `child_exit_stub_addr` in the run-game boot pipeline: the
/// stub sits just above the image's highest PT_LOAD, so no segment
/// can overwrite it, with 0 reserved as null. This harness installs
/// its own spawn loader, so nothing cross-checks the two copies.
fn exit_stub_addr_for(required: usize) -> u64 {
    (required as u64).next_multiple_of(16).max(16)
}

fn exit_stub_addr(elf: &[u8]) -> u64 {
    let required = cellgov_ppu::loader::required_memory_size(elf)
        .expect("microtest image headers parse before placement");
    exit_stub_addr_for(required)
}

fn plant_exit_stub(mem: &mut GuestMemory, addr: u64) {
    assert!(
        addr + EXIT_STUB.len() as u64 <= STACK_TOP,
        "stub at 0x{addr:x} would sit at or above the initial SP 0x{STACK_TOP:x}; \
         the image outgrew MEM_SIZE",
    );
    let range =
        ByteRange::new(GuestAddr::new(addr), EXIT_STUB.len() as u64).expect("8-byte stub range");
    mem.apply_commit(range, &EXIT_STUB)
        .expect("stub range inside the fresh region");
}

fn build_runtime(parent_elf: &[u8], child_elf: &[u8]) -> Runtime {
    let mut mem = GuestMemory::new(MEM_SIZE);
    let parent_stub = exit_stub_addr(parent_elf);
    plant_exit_stub(&mut mem, parent_stub);
    let mut state = cellgov_ppu::state::PpuState::new();
    cellgov_ppu::loader::load_ppu_elf(parent_elf, &mut mem, &mut state).expect("parent ELF loads");
    state.set_gpr(1, STACK_TOP);
    state.set_lr(parent_stub);

    let mut rt = Runtime::new(mem, Budget::new(10_000), 50_000);
    rt.registry_mut().register_with(move |id| {
        let mut unit = PpuExecutionUnit::new(id);
        *unit.state_mut() = state.clone();
        unit
    });
    rt.set_ppu_factory(|id, init| {
        let mut unit = PpuExecutionUnit::new(id);
        {
            let state = unit.state_mut();
            state.pc = init.entry_code;
            state.set_gpr(1, init.stack_top);
            state.set_gpr(2, init.entry_toc);
            state.set_gpr(3, init.arg);
            state.set_gpr(13, init.tls_base);
            state.set_lr(init.lr_sentinel);
        }
        Box::new(unit)
    });
    rt.set_process_spawn_loader(|elf_bytes, mem| {
        // Same call the run-game spawn loader makes: the content store
        // hands over whatever the title shipped, so an SCE-wrapped
        // child.self decrypts here, not at staging time.
        let plaintext = cellgov_install::self_image::to_plaintext_elf(
            elf_bytes,
            cellgov_install::self_image::KeyPolicy::AppOnly,
        )
        .map_err(|e| cellgov_core::ProcessSpawnLoadError::ImageParse {
            detail: format!("child SELF: {e}"),
        })?;
        let elf_bytes: &[u8] = &plaintext;
        // The decrypted bytes are guest-supplied here, so a header
        // that does not parse is an ImageParse refusal, not a harness
        // panic -- same shape as run-game's loader.
        let required = cellgov_ppu::loader::required_memory_size(elf_bytes).map_err(|e| {
            cellgov_core::ProcessSpawnLoadError::ImageParse {
                detail: e.to_string(),
            }
        })?;
        mem.install_region(0, MEM_SIZE, "spawned", PageSize::Page64K)
            .map_err(|source| cellgov_core::ProcessSpawnLoadError::RegionInstall { source })?;
        let child_stub = exit_stub_addr_for(required);
        plant_exit_stub(mem, child_stub);
        let mut state = cellgov_ppu::state::PpuState::new();
        cellgov_ppu::loader::load_ppu_elf(elf_bytes, mem, &mut state).map_err(|e| {
            cellgov_core::ProcessSpawnLoadError::ImageLoad {
                detail: e.to_string(),
            }
        })?;
        Ok(SpawnedProcessImage {
            entry_code: state.pc,
            entry_toc: state.gpr[2],
            stack_top: STACK_TOP,
            lr_sentinel: child_stub,
        })
    });
    rt.lv2_host_mut()
        .content_store_mut()
        .register(b"/app_home/child.self", child_elf.to_vec());
    rt
}

struct RunOutcome {
    hashes: Vec<(u64, u64)>,
    steps: usize,
    /// Why the step loop ended. `NoRunnableUnit` is the only clean
    /// finish (every unit exited); `MaxStepsExceeded` / `AllBlocked` /
    /// `TimeOverflow` mean the run died mid-flight, and the fixed
    /// result block would then hold stale or zeroed values.
    terminal: StepError,
    /// First fault any step reported, if one did. A faulted unit also
    /// funnels into `NoRunnableUnit`, so the terminal error alone
    /// cannot distinguish "finished" from "crashed".
    first_fault: Option<String>,
    result: [u32; 4],
    child_magic: Option<u32>,
    child_exit_status: Option<i32>,
    /// Named host invariant-break sites and their hit counts. The
    /// host logs these to stderr, which a passing test swallows, so
    /// the run has to carry them out for an assertion to see them.
    invariant_breaks: Vec<(&'static str, u64)>,
}

/// Invariant-break sites this microtest is known to trip, with their
/// per-run counts. `process.spawn_unconsumed_args` is the guest
/// passing sc-21 trailing register args the host does not model; the
/// gate is an equality so both a new site and a fixed one show up.
/// Entries must be in site-string order -- the run collects them from
/// a `BTreeMap`.
const EXPECTED_INVARIANT_BREAKS: &[(&str, u64)] = &[("process.spawn_unconsumed_args", 1)];

fn run_once(parent_elf: &[u8], child_elf: &[u8]) -> RunOutcome {
    let mut rt = build_runtime(parent_elf, child_elf);
    let mut hashes = Vec::new();
    let mut steps = 0usize;
    let mut first_fault: Option<String> = None;
    let terminal = loop {
        match rt.step() {
            Ok(s) => {
                if first_fault.is_none() {
                    if let Some(fault) = &s.result.fault {
                        first_fault = Some(format!("step {steps} unit {:?}: {fault:?}", s.unit));
                    }
                }
                rt.commit_step(&s.result, &s.effects)
                    .expect("microtest commits never fail validation");
                hashes.push((rt.sync_state_hash(), rt.committed_memory_hash()));
                steps += 1;
            }
            Err(e) => break e,
        }
    };
    let read_u32 = |mem: &GuestMemory, addr: u64| -> u32 {
        let range = ByteRange::new(GuestAddr::new(addr), 4).expect("fixed 4-byte range");
        let bytes = mem.read(range).expect("result region mapped");
        u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    };
    let result = [
        read_u32(rt.memory(), RESULT_ADDR),
        read_u32(rt.memory(), RESULT_ADDR + 4),
        read_u32(rt.memory(), RESULT_ADDR + 8),
        read_u32(rt.memory(), RESULT_ADDR + 12),
    ];
    let child_magic = rt
        .space_memory(AddressSpaceId::new(1))
        .ok()
        .and_then(|mem| {
            ByteRange::new(GuestAddr::new(RESULT_ADDR), 4)
                .and_then(|range| mem.read(range))
                .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        });
    let child_exit_status = rt.lv2_host().process_exit_status(EXPECTED_PID);
    let invariant_breaks = rt
        .lv2_host()
        .observability()
        .invariant_break_sites
        .iter()
        .map(|(site, count)| (*site, *count))
        .collect();
    RunOutcome {
        hashes,
        steps,
        terminal,
        first_fault,
        result,
        child_magic,
        child_exit_status,
        invariant_breaks,
    }
}

#[test]
fn parent_spawns_child_and_observes_exit_bit_identically_twice() {
    // Same skip contract as cellgov_ppu's `skip_if_missing`: only
    // ABSENCE skips (corpus optional locally, required under CI's env
    // flag). A fixture that exists but fails to read is a real
    // failure, never a skip.
    let parent_path = std::path::Path::new(PARENT_ELF);
    let child_path = std::path::Path::new(CHILD_ELF);
    if !parent_path.exists() || !child_path.exists() {
        if std::env::var("CELLGOV_REQUIRE_MICROTESTS").is_ok() {
            panic!("required microtest fixtures missing: {PARENT_ELF}, {CHILD_ELF}");
        }
        #[allow(
            clippy::print_stderr,
            reason = "test-harness diagnostic when the optional microtest corpus is absent; CELLGOV_REQUIRE_MICROTESTS=1 promotes the skip to a panic"
        )]
        {
            eprintln!("[skip] process_spawn_wait ELFs not built (see build.sh)");
        }
        return;
    }
    let parent_elf =
        std::fs::read(parent_path).expect("parent.elf exists but is unreadable; not a skip");
    let child_elf =
        std::fs::read(child_path).expect("child.elf exists but is unreadable; not a skip");

    let a = run_once(&parent_elf, &child_elf);

    // Terminal-cause gate first: without it, a run that died at
    // MaxStepsExceeded / AllBlocked / TimeOverflow before the parent
    // published would read zeroed memory, and `spawn_rc == 0` below
    // would pass vacuously.
    assert_eq!(a.first_fault, None, "no unit may fault during the run");
    assert_eq!(
        a.terminal,
        StepError::NoRunnableUnit,
        "run must end with every unit Finished (parent and child exited), \
         not a stall or the deadlock detector",
    );

    let [status, pid, spawn_rc, polls] = a.result;
    assert_eq!(spawn_rc, 0, "sc 21 must return CELL_OK");
    assert_eq!(pid, EXPECTED_PID, "kernel-minted pid written to pid_out");
    assert_eq!(
        status, 0,
        "parent status bits: 1=spawn failed, 2=poll exhausted"
    );
    assert!(
        polls > 0,
        "the child's exit must be observed by polling, not instantly"
    );
    assert_eq!(
        a.child_magic,
        Some(CHILD_MAGIC),
        "child's write lands in its own space",
    );
    assert_eq!(a.child_exit_status, Some(42), "exit status recorded");
    assert_eq!(
        a.invariant_breaks, EXPECTED_INVARIANT_BREAKS,
        "host invariant-break sites drifted; the host logs these to stderr, which a \
         passing test would otherwise swallow",
    );

    let b = run_once(&parent_elf, &child_elf);
    assert_eq!(b.first_fault, None);
    assert_eq!(b.terminal, StepError::NoRunnableUnit);
    assert_eq!(a.steps, b.steps, "step counts must match");
    assert_eq!(
        a.hashes, b.hashes,
        "sync + committed-memory hash streams must be bit-identical",
    );
    assert_eq!(a.result, b.result);
    assert_eq!(a.child_magic, b.child_magic);
    assert_eq!(a.child_exit_status, b.child_exit_status);
    assert_eq!(a.invariant_breaks, b.invariant_breaks);
}

/// The spawn loader unwraps a genuinely SCE-wrapped child SELF
/// (build.sh wraps child.elf via make_self) and the child still runs
/// to its magic write and exit 42, bit-identically across two boots.
#[test]
fn parent_spawns_a_genuinely_sce_wrapped_child_self() {
    let parent_path = std::path::Path::new(PARENT_ELF);
    let child_path = std::path::Path::new(CHILD_SELF);
    if !parent_path.exists() || !child_path.exists() {
        if std::env::var("CELLGOV_REQUIRE_MICROTESTS").is_ok() {
            panic!("required microtest fixtures missing: {PARENT_ELF}, {CHILD_SELF}");
        }
        #[allow(
            clippy::print_stderr,
            reason = "test-harness diagnostic when the optional microtest corpus is absent; CELLGOV_REQUIRE_MICROTESTS=1 promotes the skip to a panic"
        )]
        {
            eprintln!("[skip] process_spawn_wait wrapped child.self not built (see build.sh)");
        }
        return;
    }
    let parent_elf =
        std::fs::read(parent_path).expect("parent.elf exists but is unreadable; not a skip");
    let child_self =
        std::fs::read(child_path).expect("child.self exists but is unreadable; not a skip");
    // Vacuity guard: a raw ELF renamed .self would exercise nothing.
    // `get` rather than a slice index so a truncated fixture reports
    // the wrong-magic message instead of an out-of-range panic.
    assert_eq!(
        child_self.get(..4),
        Some(cellgov_ps3_abi::sce::SCE_MAGIC.as_slice()),
        "child.self must be genuinely SCE-wrapped; rebuild via build.sh's make_self step",
    );

    let a = run_once(&parent_elf, &child_self);
    assert_eq!(a.first_fault, None, "no unit may fault during the run");
    assert_eq!(a.terminal, StepError::NoRunnableUnit);
    let [status, pid, spawn_rc, polls] = a.result;
    assert_eq!(spawn_rc, 0, "sc 21 must return CELL_OK");
    assert_eq!(pid, EXPECTED_PID);
    assert_eq!(status, 0);
    assert!(polls > 0);
    assert_eq!(
        a.child_magic,
        Some(CHILD_MAGIC),
        "the decrypted child's write lands in its own space",
    );
    assert_eq!(a.child_exit_status, Some(42));
    assert_eq!(
        a.invariant_breaks, EXPECTED_INVARIANT_BREAKS,
        "the SELF path must not trip a site the plaintext path does not",
    );

    let b = run_once(&parent_elf, &child_self);
    assert_eq!(b.first_fault, None);
    assert_eq!(b.terminal, StepError::NoRunnableUnit);
    assert_eq!(a.steps, b.steps);
    assert_eq!(
        a.hashes, b.hashes,
        "wrapped-child boots must be bit-identical"
    );
    assert_eq!(a.result, b.result);
    assert_eq!(a.child_magic, b.child_magic);
    // Host-side, so the memory-hash stream above does not cover it.
    assert_eq!(a.child_exit_status, b.child_exit_status);
}

/// The two spawn paths differ only in the SELF envelope: an
/// APP-keyed child.self and the raw child.elf it wraps must produce
/// the same step count and hash stream.
#[test]
fn a_wrapped_child_and_its_plaintext_elf_run_identically() {
    let parent_path = std::path::Path::new(PARENT_ELF);
    let elf_path = std::path::Path::new(CHILD_ELF);
    let self_path = std::path::Path::new(CHILD_SELF);
    if !parent_path.exists() || !elf_path.exists() || !self_path.exists() {
        if std::env::var("CELLGOV_REQUIRE_MICROTESTS").is_ok() {
            panic!("required microtest fixtures missing: {PARENT_ELF}, {CHILD_ELF}, {CHILD_SELF}");
        }
        #[allow(
            clippy::print_stderr,
            reason = "test-harness diagnostic when the optional microtest corpus is absent; CELLGOV_REQUIRE_MICROTESTS=1 promotes the skip to a panic"
        )]
        {
            eprintln!("[skip] process_spawn_wait elf/self pair not built (see build.sh)");
        }
        return;
    }
    let parent_elf = std::fs::read(parent_path).expect("parent.elf exists but is unreadable");
    let child_elf = std::fs::read(elf_path).expect("child.elf exists but is unreadable");
    let child_self = std::fs::read(self_path).expect("child.self exists but is unreadable");

    let plain = run_once(&parent_elf, &child_elf);
    let wrapped = run_once(&parent_elf, &child_self);
    assert_eq!(
        plain.steps, wrapped.steps,
        "the SELF envelope must not change how far the run gets",
    );
    // Memory-hash component only. `committed_memory_hash` folds every
    // space, so this asserts the reassembled child maps byte-for-byte
    // onto what the plaintext ELF maps -- the vacuity guard for the
    // unwrap. The sync-state component is excluded because the LV2
    // content store hashes the registered blob itself, and the two
    // runs register different bytes under the same guest path.
    let plain_mem: Vec<u64> = plain.hashes.iter().map(|&(_, m)| m).collect();
    let wrapped_mem: Vec<u64> = wrapped.hashes.iter().map(|&(_, m)| m).collect();
    assert_eq!(
        plain_mem, wrapped_mem,
        "unwrapping a child SELF must reproduce the plaintext ELF's committed memory",
    );
    assert_eq!(plain.terminal, wrapped.terminal);
    assert_eq!(plain.first_fault, wrapped.first_fault);
    assert_eq!(plain.result, wrapped.result);
    assert_eq!(plain.child_magic, wrapped.child_magic);
    assert_eq!(plain.child_exit_status, wrapped.child_exit_status);
    assert_eq!(plain.invariant_breaks, wrapped.invariant_breaks);
}
