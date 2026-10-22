//! The event sources, the decrementer, and the guest clock of a unit.

use super::SpuState;

impl SpuState {
    /// The event sources whose channel count is nonzero now, one event
    /// bit each.
    ///
    /// [CBEA p:149 s:9.11.1] Tg follows the MFC_RdTagStat count.
    /// [CBEA p:149 s:9.11.1] Qv is the MFC SPU command queue going from full to not full: the MFC_Cmd count rising from 0.
    /// [CBEA p:149 s:9.11.1] Sn is the MFC meeting a list element with the stall-and-notify flag set.
    /// [CBEA p:150 s:9.11.1] Mb, S1, S2 and Ms follow the SPU_RdInMbox, signal-notification and MFC_WrMSSyncReq counts.
    /// [CBEA p:161 s:9.12.6] Me is the SPU_WrOutIntrMbox count changing from 0 (full) to nonzero.
    /// [CBEA p:162 s:9.12.7] Le is the SPU_WrOutMbox count changing from 0 (full) to nonzero.
    ///
    /// No source here raises A. Only privileged software raises A, with a
    /// write to SPU_PrivCntl. The `spu_mailbox_events` console capture
    /// shows that LV2 leaves A clear for an SPU thread.
    ///
    /// [CBEA p:165 s:9.12.11] the privileged attention event occurs when the A bit of SPU_PrivCntl is set to 1.
    fn event_source_levels(&self) -> u32 {
        use cellgov_ps3_abi::hw::spu::event;
        let c = &self.channels;
        let level = |on: bool, bit: u32| if on { bit } else { 0 };
        level(c.tag_status_read.is_some(), event::TG)
            | level(c.cmd_queue_free > 0, event::QV)
            | level(c.list_stall_status != 0, event::SN)
            | level(!c.in_mbox.is_empty(), event::MB)
            | level(self.signals[0].pending, event::S1)
            | level(self.signals[1].pending, event::S2)
            | level(c.mssync_tracking.is_none(), event::MS)
            | level(c.out_mbox.is_none(), event::LE)
            | level(c.out_intr_mbox.is_none(), event::ME)
    }

    /// Record the events whose source count changed from 0 to nonzero
    /// since the unit last looked.
    pub fn update_events(&mut self) {
        let levels = self.event_source_levels();
        let rising = levels & !self.channels.event_levels;
        self.channels.event_levels = levels;
        self.raise_events(rising);
        self.sample_decrementer();
    }

    /// Set the guest tick of the instruction the unit runs next.
    pub fn set_clock(&mut self, now: cellgov_time::GuestTicks) {
        self.channels.clock = now;
    }

    /// The time base at the unit's clock.
    pub fn time_base(&self) -> u64 {
        crate::decrementer::time_base_at(self.channels.clock)
    }

    /// Raise Tm when the decrementer passed zero since the unit last
    /// looked.
    ///
    /// [CBEA p:150 s:9.11.1] the decrementer event is set when the most significant bit of the count changes from 0 to 1.
    pub fn sample_decrementer(&mut self) {
        // Every later value of the count runs down from the tick of the
        // first look, so the step that takes it reads the clock.
        if self.channels.decrementer.running() && !self.channels.decrementer.started() {
            self.note_clock_read();
        }
        let now = self.time_base();
        if self.channels.decrementer.advance(now) {
            // A step at an earlier tick finds no crossing here, so the
            // step that raises Tm reads the clock.
            self.note_clock_read();
            self.raise_events(cellgov_ps3_abi::hw::spu::event::TM);
        }
    }

    /// Mark the step in flight as one whose result depends on the guest
    /// clock.
    pub fn note_clock_read(&mut self) {
        self.channels.clock_read = true;
    }

    /// Whether the decrementer can still raise Tm: the count runs and Tm
    /// is not pending yet.
    ///
    /// While it can, a look at the event state reads the clock: a step at
    /// an earlier or a later tick can find the zero crossing on the other
    /// side of it.
    pub fn tm_can_rise(&self) -> bool {
        self.channels.decrementer.running()
            && self.channels.pending_events & cellgov_ps3_abi::hw::spu::event::TM == 0
    }

    /// The guest tick at which the decrementer raises Tm, while Tm is
    /// enabled and not already pending: the deadline a parked unit wakes
    /// at.
    pub fn decrementer_event_tick(&self) -> Option<cellgov_time::GuestTicks> {
        use cellgov_ps3_abi::hw::spu::event::TM;
        let c = &self.channels;
        if c.event_mask & TM == 0 || c.pending_events & TM != 0 {
            return None;
        }
        c.decrementer.crossing_tick()
    }
}
