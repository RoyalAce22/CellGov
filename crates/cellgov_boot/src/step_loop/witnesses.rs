//! The `BENCH_*_WITNESS` block a bench boot reports after the step loop
//! stops.
//!
//! `cellgov_compare::witness_parse` scrapes these lines back. The
//! prefixes, the field spellings, and the emission order are a contract
//! with that reader. [`BenchWitnesses::read`] gathers the counts from a
//! runtime and [`BenchWitnesses::lines`] renders them; the caller picks
//! the stream.
//!
//! Line convention:
//!
//! - An inventory line renders a map, and suppresses when the map is
//!   empty.
//! - A scalar count line prints even at zero, because the zero is the
//!   finding.
//! - `BENCH_UART_WITNESS` is the exception. Its scalars share a line
//!   with the PS3AV command inventory, so a boot that sent no command
//!   emits none of them.

use std::collections::BTreeMap;

use cellgov_core::Runtime;
use cellgov_lv2::host::{SystemIpcWitness, UnsupportedSyscallWitness};

use crate::prepare::AuthorityIdSource;

/// Everything the witness block reports, read from one runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchWitnesses {
    /// `mfvrsave` executions summed over every PPU unit.
    pub mfvrsave_executed: u64,
    /// Whether any PPU unit wrote VRSAVE.
    pub vrsave_written: bool,
    /// LV2 host invariant breaks.
    pub host_invariant_breaks: u64,
    /// The same breaks, per site. Only the first break of a boot prints
    /// its detail line, so the per-site split is the only way to read
    /// the rest.
    pub invariant_break_sites: BTreeMap<&'static str, u64>,
    /// `ldarx` executions summed over every PPU unit. The four atomics
    /// are counted per mnemonic so word-width and doubleword paths
    /// report independently.
    pub ldarx: u64,
    /// `stdcx.` executions summed over every PPU unit.
    pub stdcx: u64,
    /// `lwarx` executions summed over every PPU unit.
    pub lwarx: u64,
    /// `stwcx.` executions summed over every PPU unit.
    pub stwcx: u64,
    /// Entries to the PPU's memory-fault verdict.
    pub mem_fault_arm_entries: u64,
    /// Of those, the entries routed through the unmapped-address arm.
    pub mem_fault_unmapped_routed: u64,
    /// Timer sleeps. They bypass the LV2 dispatch, so no other witness
    /// records them, and a guest sleep loop is invisible without this.
    pub timer_sleeps: u64,
    /// RSX label writes the runtime committed.
    pub rsx_label_writes: u64,
    /// RSX set-reference dispatches.
    pub rsx_set_reference: u64,
    /// `dcbz` executions summed over every PPU unit.
    pub dcbz: u64,
    /// SPU image registrations with the content store.
    pub spu_image_register: u64,
    /// `sys_spu_thread_initialize` dispatches.
    pub spu_thread_init: u64,
    /// Lightweight-mutex acquires.
    pub lwmutex_acquires: u64,
    /// Lightweight-mutex releases.
    pub lwmutex_releases: u64,
    /// Condition-variable reacquire wakes.
    pub cond_reacquires: u64,
    /// The program authority id the host served.
    pub program_authority_id: u64,
    /// Where that id came from.
    pub authid_source: AuthorityIdSource,
    /// Locks of an unknown lightweight mutex: the cellSysmodule
    /// LoadModule-failure signature a wrong system authid reintroduces.
    pub lwmutex_unknown_locks: u64,
    /// Mutex unlocks by a thread that did not own the mutex.
    pub mutex_unlock_not_owner: u64,
    /// Every non-zero immediate LV2 return, error or value, with its
    /// hit count. A code absent here was never produced this boot.
    pub dispatch_returns: BTreeMap<u64, u64>,
    /// The same codes attributed to the arm that returned them.
    pub dispatch_return_pairs: BTreeMap<(&'static str, u64), u64>,
    /// Wait-family parks by (arm, timeout in microseconds). A zero
    /// timeout waits forever.
    pub park_timeouts: BTreeMap<(&'static str, u64), u64>,
    /// Timed waits that expired with `ETIMEDOUT`, by primitive.
    pub wait_expiries: BTreeMap<&'static str, u64>,
    /// `sys_prx_register_module` calls: (calls, taking the CoreOS
    /// manual-link branch, slots linked, NIDs left unresolved).
    pub register_module: (u64, u64, u64, u64),
    /// `sys_event_port_connect_ipc` (attempts, bound). A gap is the
    /// connect-before-create race, or a producer CellGov never runs.
    pub event_port_ipc_connects: (u64, u64),
    /// Event queues registered under an IPC key.
    pub keyed_event_queues: u64,
    /// Syscalls that reached the null backend, by number.
    pub unsupported_syscalls: BTreeMap<u64, UnsupportedSyscallWitness>,
    /// System-IPC namespace production counters, both channels.
    pub system_ipc: SystemIpcWitness,
    /// PS3AV command ids the boot sent, with hit counts.
    pub uart_cids: BTreeMap<u32, u64>,
    /// The command ids no handler answered.
    pub uart_unknown_cids: BTreeMap<u32, u64>,
    /// HDMI events suppressed before the guest could receive them.
    pub uart_events_gated: u64,
    /// Reply bytes the reply ring could not hold.
    pub uart_rx_overflow_bytes: u64,
    /// Blocking receives queued behind an empty ring.
    pub uart_readers_queued: u64,
    /// Firmware PRX load misses stubbed with a real kernel id.
    pub prx_load_hle_stubs: u64,
    /// PRX loads reported `CELL_ENOENT`.
    pub prx_load_not_found: u64,
    /// Every PRX load miss, by guest-supplied path.
    pub prx_load_misses: BTreeMap<String, u64>,
    /// Where every live PPU unit stopped, in registry order.
    pub final_units: Vec<FinalUnit>,
}

/// One live PPU unit's terminal state: the parking map a `MaxSteps`
/// boot reads its idle loops from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalUnit {
    /// The unit id.
    pub unit: u64,
    /// Program counter.
    pub pc: u64,
    /// Link register.
    pub lr: u64,
    /// The unit's scheduler status; `None` when the registry holds
    /// none for it.
    pub status: Option<cellgov_exec::UnitStatus>,
    /// The unit's own `ldarx` executions, which separate a spinner from
    /// a parked unit.
    pub ldarx: u64,
    /// The unit's own `lwarx` executions.
    pub lwarx: u64,
}

impl BenchWitnesses {
    /// Read every count the block reports from `rt`.
    #[must_use]
    pub fn read(rt: &Runtime, authid_source: AuthorityIdSource) -> Self {
        let mut ppu_totals = PpuTotals::default();
        let mut final_units = Vec::new();
        for (id, unit) in rt.registry().iter() {
            if let Some(ppu) = unit
                .as_any()
                .downcast_ref::<cellgov_ppu::PpuExecutionUnit>()
            {
                let state = ppu.state();
                ppu_totals.add(state);
                final_units.push(FinalUnit {
                    unit: id.raw(),
                    pc: state.pc,
                    lr: state.lr(),
                    status: rt.registry().effective_status(id),
                    ldarx: state.ldarx_executed,
                    lwarx: state.lwarx_executed,
                });
            }
        }
        let host = rt.lv2_host();
        let obs = host.observability();
        Self {
            mfvrsave_executed: ppu_totals.mfvrsave_executed,
            vrsave_written: ppu_totals.vrsave_written,
            host_invariant_breaks: obs.invariant_break_count as u64,
            invariant_break_sites: obs.invariant_break_sites.clone(),
            ldarx: ppu_totals.ldarx,
            stdcx: ppu_totals.stdcx,
            lwarx: ppu_totals.lwarx,
            stwcx: ppu_totals.stwcx,
            mem_fault_arm_entries: ppu_totals.mem_fault_arm_entries,
            mem_fault_unmapped_routed: ppu_totals.mem_fault_unmapped_routed,
            timer_sleeps: rt.timer_sleep_dispatches(),
            rsx_label_writes: rt.rsx_label_writes_committed(),
            rsx_set_reference: rt.rsx_set_reference_dispatches(),
            dcbz: ppu_totals.dcbz,
            spu_image_register: host.content_store().register_invocations(),
            spu_thread_init: obs.spu_thread_initialize_dispatches,
            lwmutex_acquires: host.lwmutexes().acquires_count(),
            lwmutex_releases: host.lwmutexes().releases_count(),
            cond_reacquires: obs.cond_reacquire_wake_calls,
            program_authority_id: host.program_authority_id(),
            authid_source,
            lwmutex_unknown_locks: obs.lwmutex_unknown_lock_count,
            mutex_unlock_not_owner: obs.mutex_unlock_not_owner_count,
            dispatch_returns: obs.dispatch_nonzero_returns.clone(),
            dispatch_return_pairs: obs.dispatch_return_pairs.clone(),
            park_timeouts: obs.park_timeouts.clone(),
            wait_expiries: obs.wait_timeout_expiries.clone(),
            register_module: obs.prx_register_module_witness(),
            event_port_ipc_connects: obs.event_port_ipc_connects,
            keyed_event_queues: host.keyed_event_queue_count() as u64,
            unsupported_syscalls: obs.unsupported_syscalls.clone(),
            system_ipc: obs.system_ipc_witness.clone(),
            uart_cids: obs.uart_cids.clone(),
            uart_unknown_cids: obs.uart_unknown_cids.clone(),
            uart_events_gated: obs.uart_events_gated,
            uart_rx_overflow_bytes: obs.uart_rx_overflow_bytes,
            uart_readers_queued: obs.uart_readers_queued,
            prx_load_hle_stubs: obs.prx_load_hle_stub_count,
            prx_load_not_found: obs.prx_load_not_found_count,
            prx_load_misses: obs.prx_load_misses.clone(),
            final_units,
        }
    }

    /// The block's lines, in emission order, with no trailing newline.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        out.push(format!(
            "BENCH_VRSAVE_WITNESS: mfvrsave_executed={} vrsave_written={}",
            self.mfvrsave_executed, self.vrsave_written
        ));
        out.push(format!(
            "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count={}",
            self.host_invariant_breaks
        ));
        let break_sites: Vec<String> = self
            .invariant_break_sites
            .iter()
            .map(|(site, hits)| format!("{site}={hits}"))
            .collect();
        if !break_sites.is_empty() {
            out.push(format!(
                "BENCH_HOST_INVARIANT_BREAK_SITES: {}",
                break_sites.join(" ")
            ));
        }
        out.push(format!(
            "BENCH_ATOMIC_WITNESS: ldarx={} stdcx={} lwarx={} stwcx={}",
            self.ldarx, self.stdcx, self.lwarx, self.stwcx
        ));
        out.push(format!(
            "BENCH_MEM_FAULT_WITNESS: arm_entries={} unmapped_routed={}",
            self.mem_fault_arm_entries, self.mem_fault_unmapped_routed
        ));
        out.push(format!(
            "BENCH_TIMER_SLEEP_WITNESS: count={}",
            self.timer_sleeps
        ));
        out.push(format!(
            "BENCH_RSX_LABEL_WRITES_WITNESS: count={}",
            self.rsx_label_writes
        ));
        out.push(format!(
            "BENCH_RSX_SET_REFERENCE_WITNESS: count={}",
            self.rsx_set_reference
        ));
        out.push(format!("BENCH_DCBZ_WITNESS: count={}", self.dcbz));
        out.push(format!(
            "BENCH_SPU_IMAGE_REGISTER_WITNESS: count={}",
            self.spu_image_register
        ));
        out.push(format!(
            "BENCH_SPU_THREAD_INIT_WITNESS: count={}",
            self.spu_thread_init
        ));
        out.push(format!(
            "BENCH_LWMUTEX_COND_WITNESS: lwmutex_acquires={} lwmutex_releases={} cond_reacquires={}",
            self.lwmutex_acquires, self.lwmutex_releases, self.cond_reacquires
        ));
        out.push(format!(
            "BENCH_AUTHORITY_ID_WITNESS: program_authority_id=0x{:016x} authid_source={} lwmutex_unknown_locks={}",
            self.program_authority_id, self.authid_source, self.lwmutex_unknown_locks
        ));
        out.push(format!(
            "BENCH_MUTEX_UNLOCK_WITNESS: not_owner={}",
            self.mutex_unlock_not_owner
        ));
        let codes: Vec<String> = self
            .dispatch_returns
            .iter()
            .map(|(code, hits)| format!("0x{code:08x}={hits}"))
            .collect();
        if !codes.is_empty() {
            out.push(format!(
                "BENCH_DISPATCH_RETURN_WITNESS: {}",
                codes.join(" ")
            ));
        }
        let pairs: Vec<String> = self
            .dispatch_return_pairs
            .iter()
            .map(|((arm, code), hits)| format!("{arm}:0x{code:08x}={hits}"))
            .collect();
        if !pairs.is_empty() {
            out.push(format!("BENCH_DISPATCH_RETURN_PAIRS: {}", pairs.join(" ")));
        }
        let parks: Vec<String> = self
            .park_timeouts
            .iter()
            .map(|((arm, timeout), hits)| format!("{arm}:t={timeout}us={hits}"))
            .collect();
        if !parks.is_empty() {
            out.push(format!("BENCH_PARK_TIMEOUT_WITNESS: {}", parks.join(" ")));
        }
        let expiries: Vec<String> = self
            .wait_expiries
            .iter()
            .map(|(primitive, hits)| format!("{primitive}={hits}"))
            .collect();
        if !expiries.is_empty() {
            out.push(format!("BENCH_WAIT_EXPIRY_WITNESS: {}", expiries.join(" ")));
        }
        let (reg_calls, reg_manual, reg_linked, reg_unresolved) = self.register_module;
        out.push(format!(
            "BENCH_REGISTER_MODULE_WITNESS: calls={reg_calls} manual={reg_manual} linked_slots={reg_linked} unresolved_nids={reg_unresolved}"
        ));
        let (ipc_attempts, ipc_bound) = self.event_port_ipc_connects;
        out.push(format!(
            "BENCH_EVENT_PORT_WITNESS: ipc_connect_attempts={ipc_attempts} ipc_connect_bound={ipc_bound} keyed_queues={}",
            self.keyed_event_queues
        ));
        let unsupported: Vec<String> = self
            .unsupported_syscalls
            .iter()
            .map(|(number, witness)| {
                format!("{number}={}@{}", witness.hits, witness.first_hit.raw())
            })
            .collect();
        if unsupported.is_empty() {
            out.push("BENCH_UNSUPPORTED_SYSCALL_WITNESS: distinct=0".to_string());
        } else {
            out.push(format!(
                "BENCH_UNSUPPORTED_SYSCALL_WITNESS: distinct={} {}",
                unsupported.len(),
                unsupported.join(" "),
            ));
        }
        let ipc = &self.system_ipc;
        out.push(format!(
            "BENCH_SYSTEM_IPC_WITNESS: shm_creates={} shm_attaches={} shm_maps={} shm_writes={} \
             cond_creates={} cond_waits={} cond_signals={} event_queue_creates={} \
             event_queue_references={} event_queue_enqueues={} event_port_connects={} \
             distinct_keys={}",
            ipc.shm_creates,
            ipc.shm_attaches,
            ipc.shm_maps,
            ipc.shm_writes,
            ipc.cond_creates,
            ipc.cond_waits,
            ipc.cond_signals,
            ipc.event_queue_creates,
            ipc.event_queue_references,
            ipc.event_queue_enqueues,
            ipc.event_port_connects,
            ipc.keys_touched.len(),
        ));
        if !ipc.keys_touched.is_empty() {
            let inventory: Vec<String> = ipc
                .keys_touched
                .iter()
                .map(|(key, events)| format!("0x{key:016x}={events}"))
                .collect();
            out.push(format!("BENCH_SYSTEM_IPC_KEYS: {}", inventory.join(" ")));
        }
        if !self.uart_cids.is_empty() {
            let sent: Vec<String> = self
                .uart_cids
                .iter()
                .map(|(cid, hits)| format!("0x{cid:08x}={hits}"))
                .collect();
            let unknown: Vec<String> = self
                .uart_unknown_cids
                .iter()
                .map(|(cid, hits)| format!("0x{cid:08x}={hits}"))
                .collect();
            out.push(format!(
                "BENCH_UART_WITNESS: distinct={} unknown={} events_gated={} rx_overflow_bytes={} readers_queued={}",
                sent.len(),
                unknown.len(),
                self.uart_events_gated,
                self.uart_rx_overflow_bytes,
                self.uart_readers_queued,
            ));
            out.push(format!("BENCH_UART_CIDS: {}", sent.join(" ")));
            if !unknown.is_empty() {
                out.push(format!("BENCH_UART_UNKNOWN_CIDS: {}", unknown.join(" ")));
            }
        }
        out.push(format!(
            "BENCH_PRX_LOAD_WITNESS: hle_stubs={} not_found={}",
            self.prx_load_hle_stubs, self.prx_load_not_found
        ));
        // Paths are guest-supplied. Debug quoting delimits each path and
        // escapes '"' and '\', so '=' inside a path cannot be confused
        // with the '=' before the count -- but spaces inside the quotes
        // stay literal, so a consumer must extract the quoted run first;
        // whitespace-splitting alone misparses a path containing spaces.
        let prx_misses: Vec<String> = self
            .prx_load_misses
            .iter()
            .map(|(path, hits)| format!("{path:?}={hits}"))
            .collect();
        if !prx_misses.is_empty() {
            out.push(format!("BENCH_PRX_LOAD_MISSES: {}", prx_misses.join(" ")));
        }
        for u in &self.final_units {
            out.push(format!(
                "BENCH_FINAL_UNIT_WITNESS: unit={} pc=0x{:08x} lr=0x{:08x} status={} ldarx={} lwarx={}",
                u.unit,
                u.pc,
                u.lr,
                unit_status_label(u.status),
                u.ldarx,
                u.lwarx,
            ));
        }
        out
    }
}

/// Per-PPU counters summed over every unit. The sums wrap rather than
/// saturate, as the per-unit counters do.
#[derive(Default)]
struct PpuTotals {
    mfvrsave_executed: u64,
    vrsave_written: bool,
    ldarx: u64,
    stdcx: u64,
    lwarx: u64,
    stwcx: u64,
    mem_fault_arm_entries: u64,
    mem_fault_unmapped_routed: u64,
    dcbz: u64,
}

impl PpuTotals {
    fn add(&mut self, s: &cellgov_ppu::state::PpuState) {
        self.mfvrsave_executed = self.mfvrsave_executed.wrapping_add(s.mfvrsave_executed);
        self.vrsave_written |= s.vrsave_written;
        self.ldarx = self.ldarx.wrapping_add(s.ldarx_executed);
        self.stdcx = self.stdcx.wrapping_add(s.stdcx_executed);
        self.lwarx = self.lwarx.wrapping_add(s.lwarx_executed);
        self.stwcx = self.stwcx.wrapping_add(s.stwcx_executed);
        self.mem_fault_arm_entries = self
            .mem_fault_arm_entries
            .wrapping_add(s.mem_fault_arm_entries);
        self.mem_fault_unmapped_routed = self
            .mem_fault_unmapped_routed
            .wrapping_add(s.mem_fault_unmapped_routed);
        self.dcbz = self.dcbz.wrapping_add(s.dcbz_executed);
    }
}

/// Fixed spelling for the parking-map status field: the line is
/// whitespace-delimited, so the label must never contain spaces.
fn unit_status_label(status: Option<cellgov_exec::UnitStatus>) -> &'static str {
    match status {
        Some(cellgov_exec::UnitStatus::Runnable) => "runnable",
        Some(cellgov_exec::UnitStatus::Blocked) => "blocked",
        Some(cellgov_exec::UnitStatus::Faulted) => "faulted",
        Some(cellgov_exec::UnitStatus::Finished) => "finished",
        None => "none",
    }
}

#[cfg(test)]
#[path = "tests/witnesses_tests.rs"]
mod tests;
