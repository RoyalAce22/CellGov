//! The `BENCH_*_WITNESS` block a bench boot prints to stderr after the
//! step loop stops.
//!
//! `cellgov_compare::witness_parse` scrapes these lines back. The
//! prefixes, the field spellings, and the emission order are a contract
//! with that reader.
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

use cellgov_core::Runtime;

use cellgov_boot::prepare::AuthorityIdSource;

pub(super) fn print_witness_block(rt: &Runtime, authid_source: AuthorityIdSource) {
    // VRSAVE liveness witness: sum mfvrsave_executed across every
    // PPU unit so the integration gate can scrape it from stderr.
    let mut total_mfvrsave_executed: u64 = 0;
    let mut any_vrsave_written = false;
    for (_id, unit) in rt.registry().iter() {
        if let Some(ppu) = unit
            .as_any()
            .downcast_ref::<cellgov_ppu::PpuExecutionUnit>()
        {
            total_mfvrsave_executed =
                total_mfvrsave_executed.wrapping_add(ppu.state().mfvrsave_executed);
            if ppu.state().vrsave_written {
                any_vrsave_written = true;
            }
        }
    }
    eprintln!(
        "BENCH_VRSAVE_WITNESS: mfvrsave_executed={total_mfvrsave_executed} vrsave_written={any_vrsave_written}"
    );

    let host_invariant_breaks = rt.lv2_host().observability().invariant_break_count as u64;
    eprintln!("BENCH_HOST_INVARIANT_BREAKS_WITNESS: count={host_invariant_breaks}");
    // Only the first break of a boot prints its detail line, so the
    // per-site split is the only way to read the rest.
    let break_sites: Vec<String> = rt
        .lv2_host()
        .observability()
        .invariant_break_sites
        .iter()
        .map(|(site, hits)| format!("{site}={hits}"))
        .collect();
    if !break_sites.is_empty() {
        eprintln!(
            "BENCH_HOST_INVARIANT_BREAK_SITES: {}",
            break_sites.join(" ")
        );
    }

    // Per-mnemonic so word-width and doubleword paths report
    // independently.
    let mut ldarx_total: u64 = 0;
    let mut stdcx_total: u64 = 0;
    let mut lwarx_total: u64 = 0;
    let mut stwcx_total: u64 = 0;
    for (_id, unit) in rt.registry().iter() {
        if let Some(ppu) = unit
            .as_any()
            .downcast_ref::<cellgov_ppu::PpuExecutionUnit>()
        {
            ldarx_total = ldarx_total.wrapping_add(ppu.state().ldarx_executed);
            stdcx_total = stdcx_total.wrapping_add(ppu.state().stdcx_executed);
            lwarx_total = lwarx_total.wrapping_add(ppu.state().lwarx_executed);
            stwcx_total = stwcx_total.wrapping_add(ppu.state().stwcx_executed);
        }
    }
    eprintln!(
        "BENCH_ATOMIC_WITNESS: ldarx={ldarx_total} stdcx={stdcx_total} lwarx={lwarx_total} stwcx={stwcx_total}"
    );

    // MemFault witness: arm_entries counts entries to
    // ExecuteVerdict::MemFault; unmapped_routed increments only
    // inside the MemError::Unmapped arm.
    let mut mem_fault_arm_entries: u64 = 0;
    let mut mem_fault_unmapped_routed: u64 = 0;
    for (_id, unit) in rt.registry().iter() {
        if let Some(ppu) = unit
            .as_any()
            .downcast_ref::<cellgov_ppu::PpuExecutionUnit>()
        {
            mem_fault_arm_entries =
                mem_fault_arm_entries.wrapping_add(ppu.state().mem_fault_arm_entries);
            mem_fault_unmapped_routed =
                mem_fault_unmapped_routed.wrapping_add(ppu.state().mem_fault_unmapped_routed);
        }
    }
    eprintln!(
        "BENCH_MEM_FAULT_WITNESS: arm_entries={mem_fault_arm_entries} unmapped_routed={mem_fault_unmapped_routed}"
    );

    // Timer sleeps bypass Lv2Host::dispatch, so no other witness
    // records them; a guest sleep loop is invisible without this line.
    let timer_sleeps = rt.timer_sleep_dispatches();
    eprintln!("BENCH_TIMER_SLEEP_WITNESS: count={timer_sleeps}");

    let rsx_label_writes_committed = rt.rsx_label_writes_committed();
    eprintln!("BENCH_RSX_LABEL_WRITES_WITNESS: count={rsx_label_writes_committed}");

    let rsx_set_reference_dispatches = rt.rsx_set_reference_dispatches();
    eprintln!("BENCH_RSX_SET_REFERENCE_WITNESS: count={rsx_set_reference_dispatches}");

    let mut dcbz_total: u64 = 0;
    for (_id, unit) in rt.registry().iter() {
        if let Some(ppu) = unit
            .as_any()
            .downcast_ref::<cellgov_ppu::PpuExecutionUnit>()
        {
            dcbz_total = dcbz_total.wrapping_add(ppu.state().dcbz_executed);
        }
    }
    eprintln!("BENCH_DCBZ_WITNESS: count={dcbz_total}");

    let spu_image_registers = rt.lv2_host().content_store().register_invocations();
    eprintln!("BENCH_SPU_IMAGE_REGISTER_WITNESS: count={spu_image_registers}");

    let spu_thread_init_dispatches = rt
        .lv2_host()
        .observability()
        .spu_thread_initialize_dispatches;
    eprintln!("BENCH_SPU_THREAD_INIT_WITNESS: count={spu_thread_init_dispatches}");

    let lwmutex_acquires = rt.lv2_host().lwmutexes().acquires_count();
    let lwmutex_releases = rt.lv2_host().lwmutexes().releases_count();
    let cond_reacquires = rt.lv2_host().observability().cond_reacquire_wake_calls;
    eprintln!(
        "BENCH_LWMUTEX_COND_WITNESS: lwmutex_acquires={lwmutex_acquires} lwmutex_releases={lwmutex_releases} cond_reacquires={cond_reacquires}"
    );

    // lwmutex_unknown_locks is the cellSysmodule LoadModule-failure
    // signature; a wrong system authid reintroduces it.
    let program_authority_id = rt.lv2_host().program_authority_id();
    let lwmutex_unknown_locks = rt.lv2_host().observability().lwmutex_unknown_lock_count;
    eprintln!(
        "BENCH_AUTHORITY_ID_WITNESS: program_authority_id=0x{program_authority_id:016x} authid_source={authid_source} lwmutex_unknown_locks={lwmutex_unknown_locks}"
    );
    let mutex_unlock_not_owner = rt.lv2_host().observability().mutex_unlock_not_owner_count;
    eprintln!("BENCH_MUTEX_UNLOCK_WITNESS: not_owner={mutex_unlock_not_owner}");
    // Every non-zero immediate return, error or value. A code absent
    // here was never produced by LV2 this boot. Line convention:
    // inventory lines (a rendered map, like this one) suppress when
    // the map is empty; scalar count lines always print because
    // their zero is the finding; a line carrying both, like the
    // unsupported-syscall one, keeps its scalar and drops the tail.
    let codes: Vec<String> = rt
        .lv2_host()
        .observability()
        .dispatch_nonzero_returns
        .iter()
        .map(|(code, hits)| format!("0x{code:08x}={hits}"))
        .collect();
    if !codes.is_empty() {
        eprintln!("BENCH_DISPATCH_RETURN_WITNESS: {}", codes.join(" "));
    }
    // Same codes attributed to the arm that returned them.
    let pairs: Vec<String> = rt
        .lv2_host()
        .observability()
        .dispatch_return_pairs
        .iter()
        .map(|((arm, code), hits)| format!("{arm}:0x{code:08x}={hits}"))
        .collect();
    if !pairs.is_empty() {
        eprintln!("BENCH_DISPATCH_RETURN_PAIRS: {}", pairs.join(" "));
    }
    // Wait-family parks by (arm, timeout usec). timeout=0 is
    // wait-forever; nonzero registers a wake-at-guest-tick deadline.
    let parks: Vec<String> = rt
        .lv2_host()
        .observability()
        .park_timeouts
        .iter()
        .map(|((arm, timeout), hits)| format!("{arm}:t={timeout}us={hits}"))
        .collect();
    if !parks.is_empty() {
        eprintln!("BENCH_PARK_TIMEOUT_WITNESS: {}", parks.join(" "));
    }
    // Timed waits that expired with ETIMEDOUT, by primitive.
    let expiries: Vec<String> = rt
        .lv2_host()
        .observability()
        .wait_timeout_expiries
        .iter()
        .map(|(primitive, hits)| format!("{primitive}={hits}"))
        .collect();
    if !expiries.is_empty() {
        eprintln!("BENCH_WAIT_EXPIRY_WITNESS: {}", expiries.join(" "));
    }

    // sc 484 witness: how many register-module calls arrived, how
    // many took the CoreOS manual-link branch, and how the import
    // walk resolved. A frontier run with linked=0 means the branch
    // ran but bound nothing.
    let (reg_calls, reg_manual, reg_linked, reg_unresolved) =
        rt.lv2_host().observability().prx_register_module_witness();
    eprintln!(
        "BENCH_REGISTER_MODULE_WITNESS: calls={reg_calls} manual={reg_manual} linked_slots={reg_linked} unresolved_nids={reg_unresolved}"
    );

    // Event-port IPC binding: attempts vs. those that found a queue
    // registered under the key. A gap is the connect-before-create
    // race, or a producer CellGov never runs.
    let (ipc_attempts, ipc_bound) = rt.lv2_host().observability().event_port_ipc_connects;
    let keyed_queues = rt.lv2_host().keyed_event_queue_count();
    eprintln!(
        "BENCH_EVENT_PORT_WITNESS: ipc_connect_attempts={ipc_attempts} ipc_connect_bound={ipc_bound} keyed_queues={keyed_queues}"
    );

    // Null-backend inventory: which syscalls this title issued that
    // CellGov does not implement, and how often. The key set is the
    // frontier row; the counts separate a probe from a retry loop.
    let unsupported: Vec<String> = rt
        .lv2_host()
        .observability()
        .unsupported_syscalls
        .iter()
        .map(|(number, hits)| format!("{number}={hits}"))
        .collect();
    if unsupported.is_empty() {
        eprintln!("BENCH_UNSUPPORTED_SYSCALL_WITNESS: distinct=0");
    } else {
        eprintln!(
            "BENCH_UNSUPPORTED_SYSCALL_WITNESS: distinct={} {}",
            unsupported.len(),
            unsupported.join(" "),
        );
    }

    // System-IPC namespace production witnesses, both channels. A
    // silent namespace prints all zeros; the key line is suppressed
    // rather than printed empty.
    let ipc = &rt.lv2_host().observability().system_ipc_witness;
    eprintln!(
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
    );
    if !ipc.keys_touched.is_empty() {
        let inventory: Vec<String> = ipc
            .keys_touched
            .iter()
            .map(|(key, events)| format!("0x{key:016x}={events}"))
            .collect();
        eprintln!("BENCH_SYSTEM_IPC_KEYS: {}", inventory.join(" "));
    }

    // Virtual-UART witnesses: the PS3AV command inventory the boot
    // sent, the ids nothing answered, and the events and bytes the
    // AV manager could not deliver.
    let obs = rt.lv2_host().observability();
    if !obs.uart_cids.is_empty() {
        let sent: Vec<String> = obs
            .uart_cids
            .iter()
            .map(|(cid, hits)| format!("0x{cid:08x}={hits}"))
            .collect();
        let unknown: Vec<String> = obs
            .uart_unknown_cids
            .iter()
            .map(|(cid, hits)| format!("0x{cid:08x}={hits}"))
            .collect();
        eprintln!(
            "BENCH_UART_WITNESS: distinct={} unknown={} events_gated={} rx_overflow_bytes={} readers_queued={}",
            sent.len(),
            unknown.len(),
            obs.uart_events_gated,
            obs.uart_rx_overflow_bytes,
            obs.uart_readers_queued,
        );
        eprintln!("BENCH_UART_CIDS: {}", sent.join(" "));
        if !unknown.is_empty() {
            eprintln!("BENCH_UART_UNKNOWN_CIDS: {}", unknown.join(" "));
        }
    }

    // PRX load-miss witnesses: firmware misses stubbed with a real
    // kernel id vs loads reported CELL_ENOENT. Non-vacuity evidence
    // for the sc 480 miss arms.
    let prx_hle_stubs = rt.lv2_host().observability().prx_load_hle_stub_count;
    let prx_not_found = rt.lv2_host().observability().prx_load_not_found_count;
    eprintln!("BENCH_PRX_LOAD_WITNESS: hle_stubs={prx_hle_stubs} not_found={prx_not_found}");
    // Paths are guest-supplied. Debug quoting delimits each path and
    // escapes '"' and '\', so '=' inside a path cannot be confused
    // with the '=' before the count -- but spaces inside the quotes
    // stay literal, so a consumer must extract the quoted run first;
    // whitespace-splitting alone misparses a path containing spaces.
    let prx_misses: Vec<String> = rt
        .lv2_host()
        .observability()
        .prx_load_misses
        .iter()
        .map(|(path, hits)| format!("{path:?}={hits}"))
        .collect();
    if !prx_misses.is_empty() {
        eprintln!("BENCH_PRX_LOAD_MISSES: {}", prx_misses.join(" "));
    }

    // Terminal parking map: where every live PPU unit stopped, its
    // scheduler status, and its per-unit atomic traffic. On a
    // MaxSteps boot this names the PCs an idle loop spins at; the
    // per-unit ldarx split separates the spinners from the parked.
    for (id, unit) in rt.registry().iter() {
        if let Some(ppu) = unit
            .as_any()
            .downcast_ref::<cellgov_ppu::PpuExecutionUnit>()
        {
            let status = unit_status_label(rt.registry().effective_status(id));
            eprintln!(
                "BENCH_FINAL_UNIT_WITNESS: unit={} pc=0x{:08x} lr=0x{:08x} status={status} ldarx={} lwarx={}",
                id.raw(),
                ppu.state().pc,
                ppu.state().lr(),
                ppu.state().ldarx_executed,
                ppu.state().lwarx_executed,
            );
        }
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
