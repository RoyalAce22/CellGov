//! [`Runtime::step`] -- select a unit, grant budget, run it to yield,
//! advance guest time, emit trace records.

use cellgov_event::UnitId;
use cellgov_exec::ExecutionContext;
use cellgov_time::GuestTicks;
use cellgov_trace::TraceRecord;

use crate::runtime::state::Runtime;
use crate::runtime::trace_bridge::traced_barrier_kind;
use crate::runtime::trace_bridge::traced_effect_kind;
use crate::runtime::trace_bridge::traced_yield_reason;
use crate::runtime::types::{RuntimeMode, RuntimeStep, StepError};

impl Runtime {
    /// True when a unit is runnable, or when one is parked and a queued
    /// DMA completion or timer deadline can wake it through the time
    /// warp.
    ///
    /// False is the sound half: [`Runtime::step`] then answers
    /// [`StepError::NoRunnableUnit`] or [`StepError::AllBlocked`], and
    /// the execution is maximal. True says a unit is there to run and
    /// no more than that. `step` keeps its own refusals --
    /// [`StepError::MaxStepsExceeded`] and
    /// [`StepError::SchedulerNotReinstalled`] -- and a deadline that
    /// fires without waking anyone still ends in `AllBlocked`.
    ///
    /// A caller bounding a run reads this before it reports its own
    /// bound, so the bound it reports is one the work justified. The
    /// answer reads the registry the scheduler selects from rather than
    /// the installed scheduler: [`crate::Scheduler::select_next`]
    /// answers `None` where no unit is runnable, so a stale prescribed
    /// choice left over from the previous step cannot narrow it.
    ///
    /// It sits beside [`Runtime::step`] because it is a second reading
    /// of the question that function answers, and a wake source added
    /// there has to be added here.
    pub fn can_take_another_step(&self) -> bool {
        // A PPU thread off the PPE takes a hardware thread the next step
        // finds free.
        if self.registry.runnable_ids().next().is_some() || self.registry.any_off_core() {
            return true;
        }
        let any_blocked = self.registry.ids().any(|id| {
            self.registry.effective_status(id) == Some(cellgov_exec::UnitStatus::Blocked)
        });
        any_blocked
            && (self.dma_queue.next_event_time().is_some()
                || self.timer_wakes.peek_deadline().is_some())
    }

    /// Select a unit, grant budget, run it to yield, advance guest time.
    ///
    /// # Errors
    ///
    /// - [`StepError::MaxStepsExceeded`] -- deadlock detector tripped.
    /// - [`StepError::NoRunnableUnit`] -- terminal stall (nothing can wake).
    /// - [`StepError::AllBlocked`] -- at least one unit parked, none runnable.
    /// - [`StepError::TimeOverflow`] -- consumed budget would push guest
    ///   time past `u64::MAX`.
    ///
    /// Emitted effects are returned verbatim in [`RuntimeStep::result`];
    /// [`Runtime::commit_step`] drives the commit pipeline over them.
    pub fn step(&mut self) -> Result<RuntimeStep, StepError> {
        if self.scheduler_dirty_after_restore {
            return Err(StepError::SchedulerNotReinstalled);
        }
        if self.steps_taken >= self.max_steps {
            return Err(StepError::MaxStepsExceeded);
        }
        // The time warp below fires the DMA and timer wakes itself and
        // lands their guest writes, so the clear runs ahead of it.
        self.last_host_writes.clear();
        self.last_lv2_effects.clear();
        self.last_problem_state_accesses.clear();
        self.last_mailbox_read = None;

        // A PPU thread woken, parked or yielding since the last commit
        // changes which ones hold the PPE.
        self.place_ppe_threads();
        // A unit registered or woken since the last commit takes its
        // clock from that commit's tick.
        self.registry.sync_clocks(self.time);
        // A commit leaves an event unfired while a runnable unit's clock
        // is behind its tick. The event fires here once every runnable
        // clock reaches that tick.
        let lowest = match self.next_event_time() {
            Some(_) => self.registry.lowest_runnable_clock(),
            None => None,
        };
        if let Some(lowest) = lowest {
            let (due, timer_due) = self.fire_due_events(lowest);
            if !due.is_empty() || !timer_due.is_empty() {
                // A PPU thread these events woke takes a hardware thread
                // only by the placement rule.
                self.place_ppe_threads();
                self.registry.sync_clocks(self.time);
                self.emit_time_warp_trace(&due, &timer_due);
            }
        }

        let unit_id = match self.scheduler.select_next(&self.registry) {
            Some(id) => id,
            None => {
                // All units blocked: time-warp to the earlier of the
                // next pending DMA completion and the next timer-wake
                // deadline. Both firing paths wake their unit via
                // set_status_override(Runnable) so the retry can
                // schedule it. A fired deadline can wake nobody (a
                // contended cond expiry re-parks its waiter on the
                // mutex untimed), so the warp loops while a wake
                // source remains instead of reporting a terminal
                // AllBlocked with live future deadlines still queued.
                // Each pass pops at least the queue head it warped
                // to, so the loop terminates.
                let any_blocked = self.registry.ids().any(|id| {
                    self.registry.effective_status(id) == Some(cellgov_exec::UnitStatus::Blocked)
                });
                if !any_blocked {
                    return Err(StepError::NoRunnableUnit);
                }
                loop {
                    let Some(next_time) = self.next_event_time() else {
                        return Err(StepError::AllBlocked);
                    };
                    self.time = next_time;
                    let (due, timer_due) = self.fire_due_events(next_time);
                    self.wake_reservation_lost_waiters();
                    self.place_ppe_threads();
                    self.registry.sync_clocks(self.time);
                    self.emit_time_warp_trace(&due, &timer_due);
                    if let Some(id) = self.scheduler.select_next(&self.registry) {
                        break id;
                    }
                }
            }
        };
        let start = self.registry.clock(unit_id);

        // A warp above can widen the set the scheduler chose from, so
        // record it here.
        self.last_runnable.clear();
        self.last_runnable.extend(self.registry.runnable_ids());
        let (ppe_siblings, ppe_sibling_pressure) = self.ppe_siblings(unit_id);
        self.last_ppe_pressure = self
            .registry
            .get(unit_id)
            .filter(|u| u.shares_ppe_issue())
            .map(|u| (unit_id, u.ppe_issue_pressure()));
        self.registry.clear_status_override(unit_id);
        // A unit that runs again left its stall, so a deadline its own
        // clock set for that stall wakes nothing.
        self.timer_wakes.cancel_unit_event(unit_id);

        if self.mode == RuntimeMode::FullTrace {
            self.trace.record(&TraceRecord::UnitScheduled {
                unit: unit_id,
                granted_budget: self.budget_per_step,
                time: start,
                epoch: self.epoch,
            });
        }

        // Memory borrow scoped to `run_until_yield` to enforce the
        // freeze-during-step rule.
        self.last_entry_reservation = self
            .registry
            .get(unit_id)
            .and_then(|unit| unit.local_reservation());
        let received = self.registry.drain_receives(unit_id);
        let syscall_ret = self.registry.drain_syscall_return(unit_id);
        let reg_writes = self.registry.drain_register_writes(unit_id);
        let mut effects_buf = std::mem::take(&mut self.effects_buf);
        effects_buf.clear();
        let mailbox_read = core::cell::Cell::new(false);
        let own_mailbox = cellgov_sync::MailboxId::new(unit_id.raw());
        let (result, retired_hashes, retired_full, spu_hashes, spu_full, barriers) = {
            let unit_mem =
                crate::runtime::spaces::resolve_unit_memory(&self.memory, &self.spaces, unit_id);
            let ctx = if let Some(code) = syscall_ret {
                if reg_writes.is_empty() {
                    ExecutionContext::with_syscall_return(unit_mem, &received, code)
                } else {
                    ExecutionContext::with_syscall_return_and_regs(
                        unit_mem,
                        &received,
                        code,
                        &reg_writes,
                    )
                }
            } else {
                ExecutionContext::with_received(unit_mem, &received)
            };
            let (queued_commands, outstanding_tags) = self.unit_dma_view(unit_id);
            let (next_sequence, oldest_transfer) = self.mfc_transfer_view(unit_id);
            let unit_reservations = crate::runtime::spaces::resolve_unit_reservations(
                &self.reservations,
                &self.spaces,
                unit_id,
            );
            // A unit's own inbound mailbox shares its id. An empty
            // mailbox collects into a Vec that does not allocate.
            let inbound_mailbox: Vec<u32> = self
                .mailbox_registry
                .get(own_mailbox)
                .map_or_else(Vec::new, |mailbox| mailbox.iter().copied().collect());
            let ctx = ctx
                .with_reservations(unit_reservations)
                .with_current_tick(start)
                .with_trace_per_step(self.mode != RuntimeMode::FaultDriven)
                .with_outstanding_dma_tags(outstanding_tags)
                .with_list_stall_tags(self.dma_queue.stall_notify_tags(unit_id))
                .with_dma_queue_occupancy(queued_commands)
                .with_mfc_transfer_view(next_sequence, oldest_transfer)
                .with_inbound_mailbox(&inbound_mailbox)
                .with_mailbox_read_flag(&mailbox_read)
                .with_ppe_siblings(ppe_siblings)
                .with_ppe_sibling_pressure(ppe_sibling_pressure);
            let unit = self
                .registry
                .get_mut(unit_id)
                .expect("scheduler returned an id that is not in the registry");
            let res = unit.run_until_yield(self.budget_per_step, &ctx, &mut effects_buf);
            let (retired_hashes, retired_full, spu_hashes, spu_full, barriers) =
                if self.mode == RuntimeMode::FaultDriven {
                    (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new())
                } else {
                    (
                        unit.drain_retired_state_hashes(),
                        unit.drain_retired_state_full(),
                        unit.drain_retired_spu_state_hashes(),
                        unit.drain_retired_spu_state_full(),
                        unit.drain_barriers(),
                    )
                };
            (
                res,
                retired_hashes,
                retired_full,
                spu_hashes,
                spu_full,
                barriers,
            )
        };
        if mailbox_read.get() && self.mailbox_registry.get(own_mailbox).is_some() {
            self.last_mailbox_read = Some(own_mailbox);
        }

        // PpuStateHash and PpuStateFull pair by step index so the diff
        // printer matches a hash divergence with its full-state snapshot.
        for (pc, hash) in retired_hashes {
            self.trace.record(&TraceRecord::PpuStateHash {
                step: self.per_step_index,
                pc,
                hash: cellgov_trace::StateHash::new(hash),
            });
            self.per_step_index += 1;
        }
        // The unit stamps each snapshot with its retirement counter at
        // capture, so mid-run windows keep true step attribution. For a
        // single unit traced from boot this equals the hash stream's
        // step index; with multiple traced units the hash stream
        // interleaves globally and snapshots match by (step, pc).
        for (step, pc, fingerprint) in retired_full {
            let cellgov_exec::PpuFingerprint {
                gpr,
                lr,
                ctr,
                xer,
                cr,
                reservation_line,
            } = fingerprint;
            self.zoom_trace.record(&TraceRecord::PpuStateFull {
                step,
                pc,
                gpr,
                lr,
                ctr,
                xer,
                cr,
                reservation_line,
            });
        }
        // An SPU record names its unit and counts steps per unit, so a
        // hash and its full state pair by (unit, step).
        for (step, pc, hash) in spu_hashes {
            self.trace.record(&TraceRecord::SpuStateHash {
                unit: unit_id,
                step,
                pc,
                hash: cellgov_trace::StateHash::new(hash),
            });
        }
        for (step, pc, fingerprint) in spu_full {
            let cellgov_exec::SpuFingerprint {
                regs,
                fpscr,
                lslr,
                interrupts_enabled,
                srr0,
                reservation_line,
            } = fingerprint;
            self.zoom_trace.record(&TraceRecord::SpuStateFull {
                unit: unit_id,
                step,
                pc,
                fpscr,
                lslr,
                interrupts_enabled,
                srr0,
                reservation_line,
            });
            for (block, chunk) in regs.chunks_exact(16).enumerate() {
                let mut block_regs = [0u128; 16];
                block_regs.copy_from_slice(chunk);
                self.zoom_trace.record(&TraceRecord::SpuRegisters {
                    unit: unit_id,
                    step,
                    first: (block * 16) as u8,
                    regs: block_regs,
                });
            }
        }
        for barrier in barriers {
            self.trace.record(&TraceRecord::Barrier {
                unit: unit_id,
                pc: barrier.pc,
                kind: traced_barrier_kind(barrier.kind),
            });
        }

        let advance: GuestTicks = result.consumed_cost.into();
        let time_after = start.checked_add(advance).ok_or(StepError::TimeOverflow)?;
        // The commit that follows stamps this step's effects with the
        // unit's clock at the end of the step.
        self.registry.set_clock(unit_id, time_after);
        self.time = time_after;
        self.steps_taken += 1;
        self.last_scheduled_unit = Some(unit_id);
        self.drain_provisional_reads_to_trace(unit_id);

        if let Some(tap) = self.tap.as_deref_mut() {
            // The drain above emptied the provisional-read log. A
            // reserved-region read by the observer would otherwise trace
            // at the next commit as a read by this step's unit. It would
            // also raise the run's provisional-read count.
            let step = self.steps_taken as u64;
            self.memory
                .with_reads_unlogged(|memory| tap.step(step, memory));
        }

        if self.mode == RuntimeMode::FullTrace {
            self.trace.record(&TraceRecord::StepCompleted {
                unit: unit_id,
                yield_reason: traced_yield_reason(
                    result.yield_reason,
                    result.local_diagnostics.syscall_lev,
                ),
                consumed_cost: result.consumed_cost,
                time_after,
            });

            for (sequence, effect) in effects_buf.iter().enumerate() {
                self.trace.record(&TraceRecord::EffectEmitted {
                    unit: unit_id,
                    sequence: sequence as u32,
                    kind: traced_effect_kind(effect),
                });
            }
        }

        // Fresh empty Vec avoids allocating in the common zero-effects
        // FaultDriven case.
        self.effects_buf = Vec::new();
        Ok(RuntimeStep {
            unit: unit_id,
            result,
            effects: effects_buf,
            time_after,
            epoch_after: self.epoch,
        })
    }

    /// The number of PPE threads other than `unit` in the runnable set
    /// the step starts from, and the largest issue pressure among them;
    /// both zero when `unit` is not a PPE thread.
    ///
    /// [CBE-Handbook p:754 s:A.6.1] the threads take dispatch in turn, a
    /// thread every other cycle.
    ///
    /// [CBE-Handbook p:301 s:10.2.3.1] the threads share the issue stall
    /// point: a stall at IS2 stalls both threads.
    fn ppe_siblings(&self, unit: UnitId) -> (u32, u32) {
        let ppe = |id: UnitId| self.registry.get(id).filter(|u| u.shares_ppe_issue());
        if ppe(unit).is_none() {
            return (0, 0);
        }
        let mut others = 0u32;
        let mut pressure = 0;
        for sibling in self
            .last_runnable
            .iter()
            .filter(|&&id| id != unit)
            .filter_map(|&id| ppe(id))
        {
            others = others.saturating_add(1);
            pressure = pressure.max(sibling.ppe_issue_pressure());
        }
        (others, pressure)
    }

    /// Trace the reserved-zero reads every address space logged since
    /// the previous drain, attributed to `unit` at the current step.
    ///
    /// Drained in every mode so the per-memory log never outlives one
    /// step; recorded only when the run is tracing.
    pub(super) fn drain_provisional_reads_to_trace(&mut self, unit: UnitId) {
        let mut reads = Vec::new();
        for (_, mem) in self.address_spaces() {
            reads.extend(mem.drain_provisional_reads());
        }
        if self.mode == RuntimeMode::FaultDriven {
            return;
        }
        let step = self.steps_taken as u64;
        for read in reads {
            self.trace.record(&TraceRecord::ReservedRegionRead {
                unit,
                step,
                addr: read.addr,
                len: read.len,
                hits: read.hits,
            });
        }
    }
}
