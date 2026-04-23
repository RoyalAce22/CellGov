//! sys_rsx LV2 dispatch.
//!
//! Unimplemented handlers return `CELL_ENOSYS` so a guest that
//! reaches the syscall surface sees a deterministic "not
//! implemented" rather than a CELL_OK masquerade.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::{ByteRange, GuestAddr};

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;

/// Fixed `context_id` returned from `sys_rsx_context_allocate`.
/// RPCS3 `sys_rsx.cpp:332`.
pub const RSX_CONTEXT_ID: u32 = 0x5555_5555;

/// Offsets from the sys_rsx memory base for the three kernel regions.
/// RPCS3 `sys_rsx.cpp:264-266`.
pub const DMA_CONTROL_OFFSET: u32 = 0x0000_0000;
/// Driver-info region offset.
pub const DRIVER_INFO_OFFSET: u32 = 0x0010_0000;
/// Reports region offset.
pub const REPORTS_OFFSET: u32 = 0x0020_0000;

/// Bytes reserved per sys_rsx context. RPCS3 `sys_rsx.cpp:255`
/// (`area->alloc(0x300000)`).
pub const RSX_CONTEXT_RESERVATION: u32 = 0x0030_0000;

/// Size of the reports region (semaphore + notify + report).
pub const RSX_REPORTS_SIZE: usize = 0x9400;

/// Size of the driver info region. RPCS3 `sys_rsx.h:59`.
pub const RSX_DRIVER_INFO_SIZE: usize = 0x12F8;

/// RPCS3 values populated at `sys_rsx_context_allocate`. Source:
/// `sys_rsx.cpp:293-302`. Hand-maintained.
pub mod driver_info_init {
    /// Driver version word.
    pub const VERSION_DRIVER: u32 = 0x211;
    /// GPU version word.
    pub const VERSION_GPU: u32 = 0x5c;
    /// nvcore frequency in Hz.
    pub const NVCORE_FREQUENCY: u32 = 500_000_000;
    /// Memory frequency in Hz.
    pub const MEMORY_FREQUENCY: u32 = 650_000_000;
    /// Offset from reports_base to the notify array.
    pub const REPORTS_NOTIFY_OFFSET: u32 = 0x1000;
    /// Offset from reports_base to the semaphore block.
    pub const REPORTS_OFFSET_FIELD: u32 = 0;
    /// Offset from reports_base to the report entries.
    pub const REPORTS_REPORT_OFFSET: u32 = 0x1400;
    /// Hardware channel (games = 1, VSH = 0).
    pub const HARDWARE_CHANNEL: u32 = 1;
    /// Default local RSX memory exposed to games. Matches the
    /// historical CellGov cellGcmSys setting and RPCS3's
    /// post-firmware-reservation value.
    pub const MEMORY_SIZE: u32 = 0x0F90_0000;
}

/// Semaphore init sentinel pattern. RPCS3 `sys_rsx.cpp:274-280`
/// writes this 4-u32 group repeatedly across the 1024 slots.
pub const SEMAPHORE_INIT_PATTERN: [u32; 4] = [0x1337_C0D3, 0x1337_BABE, 0x1337_BEEF, 0x1337_F001];

/// Offset of the `handler_queue` field within `RsxDriverInfo`.
/// RPCS3 `sys_rsx.h:47`.
pub const DRIVER_INFO_HANDLER_QUEUE_OFFSET: usize = 0x12D0;

/// Fill `buf` with the bytes `sys_rsx_context_allocate` writes into
/// the driver-info region. Mirrors RPCS3 `sys_rsx.cpp:291-325`:
/// memset to zero, then stamp the 10 RPCS3-supplied fields plus the
/// `handler_queue` handle created via the event-queue primitives.
///
/// # Panics
///
/// Panics if `buf.len() != RSX_DRIVER_INFO_SIZE`.
pub fn write_rsx_driver_info_init(
    buf: &mut [u8],
    memory_size: u32,
    system_mode: u32,
    handler_queue: u32,
) {
    assert_eq!(
        buf.len(),
        RSX_DRIVER_INFO_SIZE,
        "write_rsx_driver_info_init expects an RSX_DRIVER_INFO_SIZE-byte buffer"
    );
    buf.fill(0);
    let mut put = |offset: usize, value: u32| {
        buf[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    };
    put(0x00, driver_info_init::VERSION_DRIVER);
    put(0x04, driver_info_init::VERSION_GPU);
    put(0x08, memory_size);
    put(0x0C, driver_info_init::HARDWARE_CHANNEL);
    put(0x10, driver_info_init::NVCORE_FREQUENCY);
    put(0x14, driver_info_init::MEMORY_FREQUENCY);
    put(0x2C, driver_info_init::REPORTS_NOTIFY_OFFSET);
    put(0x30, driver_info_init::REPORTS_OFFSET_FIELD);
    put(0x34, driver_info_init::REPORTS_REPORT_OFFSET);
    put(0x50, system_mode);
    put(DRIVER_INFO_HANDLER_QUEUE_OFFSET, handler_queue);
}

/// Default event-queue size for the RSX handler queue. RPCS3
/// `sys_rsx.cpp:324` passes 0x20.
pub const RSX_EVENT_QUEUE_SIZE: u32 = 0x20;

/// sys_rsx_context_attribute package id: set flip mode (vsync /
/// hsync). RPCS3 `sys_rsx.cpp:545`.
pub const PACKAGE_FLIP_MODE: u32 = 0x101;

/// sys_rsx_context_attribute package id: trigger a flip buffer.
/// RPCS3 `sys_rsx.cpp:550`.
pub const PACKAGE_FLIP_BUFFER: u32 = 0x102;

/// sys_rsx_context_attribute package id: record display-buffer
/// metadata. RPCS3 `sys_rsx.cpp:624`.
pub const PACKAGE_SET_DISPLAY_BUFFER: u32 = 0x104;

/// Maximum number of display buffer slots.
pub const DISPLAY_BUFFER_COUNT: usize = 8;

/// Per-slot display buffer metadata. Fields decoded from the a4/a5
/// payload of package 0x104.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RsxDisplayBuffer {
    /// Guest-memory offset of the buffer.
    pub offset: u32,
    /// Row pitch in bytes.
    pub pitch: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// CellGov-internal package id for registering the flip-handler
/// callback. Distinct from RPCS3's 0x108 (which is vblank
/// frequency) to avoid collision if future guest-visible sys_rsx
/// service lands.
pub const PACKAGE_CELLGOV_SET_FLIP_HANDLER: u32 = 0x8000_0108;
/// CellGov-internal package id for registering the vblank-handler
/// callback.
pub const PACKAGE_CELLGOV_SET_VBLANK_HANDLER: u32 = 0x8000_010C;
/// CellGov-internal package id for registering the user-handler
/// callback.
pub const PACKAGE_CELLGOV_SET_USER_HANDLER: u32 = 0x8000_010D;

/// Fill `buf` with the bytes `sys_rsx_context_allocate` writes into
/// the reports region on real PS3. Mirrors RPCS3 `sys_rsx.cpp:269-287`:
/// memset to zero, then semaphore sentinels, notify timestamps,
/// report fields.
///
/// # Panics
///
/// Panics if `buf.len() != RSX_REPORTS_SIZE`.
pub fn write_rsx_reports_init(buf: &mut [u8]) {
    assert_eq!(
        buf.len(),
        RSX_REPORTS_SIZE,
        "write_rsx_reports_init expects an RSX_REPORTS_SIZE-byte buffer"
    );
    buf.fill(0);

    for i in 0..1024 {
        let offset = i * 4;
        buf[offset..offset + 4].copy_from_slice(&SEMAPHORE_INIT_PATTERN[i % 4].to_be_bytes());
    }

    let ts_be = u64::MAX.to_be_bytes();
    for i in 0..64 {
        let offset = 0x1000 + i * 16;
        buf[offset..offset + 8].copy_from_slice(&ts_be);
    }

    let pad_be = u32::MAX.to_be_bytes();
    for i in 0..2048 {
        let offset = 0x1400 + i * 16;
        buf[offset..offset + 8].copy_from_slice(&ts_be);
        buf[offset + 12..offset + 16].copy_from_slice(&pad_be);
    }
}

/// Per-context bookkeeping. Populated by
/// `sys_rsx_context_allocate` and consumed by subsequent handlers.
/// One instance per [`Lv2Host`]; multi-context support would require
/// keying by `context_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SysRsxContext {
    /// Whether `sys_rsx_context_allocate` has fired.
    pub allocated: bool,
    /// Context id returned to the guest (0x5555_5555 when set).
    pub context_id: u32,
    /// DMA control region guest address.
    pub dma_control_addr: u32,
    /// Driver info region guest address.
    pub driver_info_addr: u32,
    /// Reports region guest address.
    pub reports_addr: u32,
    /// Event-queue handle.
    pub event_queue_id: u32,
    /// Event-port handle.
    pub event_port_id: u32,
    /// Memory handle the context was allocated against.
    pub mem_ctx: u64,
    /// System-mode flag word passed in by the caller.
    pub system_mode: u64,
    /// Guest address returned by the most recent
    /// sys_rsx_memory_allocate. Consumed by
    /// sys_rsx_context_allocate as the base for its three-region
    /// layout, in place of bumping `rsx_mem_alloc_ptr` a second
    /// time; zero means memory_allocate has not fired.
    pub pending_mem_addr: u32,
    /// Flip-handler callback OPD address (0 = unregistered).
    pub flip_handler_addr: u32,
    /// Vblank-handler callback OPD address (0 = unregistered).
    pub vblank_handler_addr: u32,
    /// User-handler callback OPD address (0 = unregistered).
    pub user_handler_addr: u32,
    /// Display-buffer metadata slots. Slots `0..display_buffers_count`
    /// are populated; the rest are default-zero.
    pub display_buffers: [RsxDisplayBuffer; DISPLAY_BUFFER_COUNT],
    /// Count of populated display-buffer slots (monotonic; matches
    /// RPCS3 sys_rsx.cpp:646).
    pub display_buffers_count: u32,
    /// Flip mode flag (1 = hsync, 2 = vsync). 0 until FLIP_MODE fires.
    pub flip_mode: u32,
}

impl SysRsxContext {
    /// Pristine state.
    #[inline]
    pub const fn new() -> Self {
        Self {
            allocated: false,
            context_id: 0,
            dma_control_addr: 0,
            driver_info_addr: 0,
            reports_addr: 0,
            event_queue_id: 0,
            event_port_id: 0,
            mem_ctx: 0,
            system_mode: 0,
            pending_mem_addr: 0,
            flip_handler_addr: 0,
            vblank_handler_addr: 0,
            user_handler_addr: 0,
            display_buffers: [RsxDisplayBuffer {
                offset: 0,
                pitch: 0,
                width: 0,
                height: 0,
            }; DISPLAY_BUFFER_COUNT],
            display_buffers_count: 0,
            flip_mode: 0,
        }
    }

    /// FNV-1a hash over every field.
    pub fn state_hash(&self) -> u64 {
        let Self {
            allocated,
            context_id,
            dma_control_addr,
            driver_info_addr,
            reports_addr,
            event_queue_id,
            event_port_id,
            mem_ctx,
            system_mode,
            pending_mem_addr,
            flip_handler_addr,
            vblank_handler_addr,
            user_handler_addr,
            display_buffers,
            display_buffers_count,
            flip_mode,
        } = *self;
        let mut h = cellgov_mem::Fnv1aHasher::new();
        h.write(&[u8::from(allocated)]);
        h.write(&context_id.to_le_bytes());
        h.write(&dma_control_addr.to_le_bytes());
        h.write(&driver_info_addr.to_le_bytes());
        h.write(&reports_addr.to_le_bytes());
        h.write(&event_queue_id.to_le_bytes());
        h.write(&event_port_id.to_le_bytes());
        h.write(&mem_ctx.to_le_bytes());
        h.write(&system_mode.to_le_bytes());
        h.write(&pending_mem_addr.to_le_bytes());
        h.write(&flip_handler_addr.to_le_bytes());
        h.write(&vblank_handler_addr.to_le_bytes());
        h.write(&user_handler_addr.to_le_bytes());
        h.write(&display_buffers_count.to_le_bytes());
        h.write(&flip_mode.to_le_bytes());
        for buf in display_buffers.iter() {
            h.write(&buf.offset.to_le_bytes());
            h.write(&buf.pitch.to_le_bytes());
            h.write(&buf.width.to_le_bytes());
            h.write(&buf.height.to_le_bytes());
        }
        h.finish()
    }
}

impl Default for SysRsxContext {
    fn default() -> Self {
        Self::new()
    }
}

impl Lv2Host {
    /// sys_rsx_memory_allocate (665). Bump-allocates `size` bytes
    /// out of the sys_rsx region and writes `mem_handle` (u32 BE)
    /// and `mem_addr` (u64 BE) into the guest out-pointers.
    ///
    /// Returns `CELL_ENOMEM` if `size == 0`, if the requested size
    /// would wrap the cursor, or if the resulting end address
    /// exceeds [`Lv2Host::SYS_RSX_MEM_END`].
    pub(super) fn dispatch_sys_rsx_memory_allocate(
        &mut self,
        mem_handle_ptr: u32,
        mem_addr_ptr: u32,
        size: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        if size == 0 {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ENOMEM.into(),
                effects: vec![],
            };
        }
        let Some(end) = self.rsx_mem_alloc_ptr.checked_add(size) else {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ENOMEM.into(),
                effects: vec![],
            };
        };
        if end > Self::SYS_RSX_MEM_END {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ENOMEM.into(),
                effects: vec![],
            };
        }

        let handle = self.rsx_mem_handle_counter;
        let addr = self.rsx_mem_alloc_ptr;
        self.rsx_mem_alloc_ptr = end;
        self.rsx_mem_handle_counter = handle.wrapping_add(1);
        // Record the base so a subsequent sys_rsx_context_allocate
        // reuses this reservation instead of bumping the cursor
        // again. RPCS3 does its real reservation inside
        // context_allocate; CellGov pre-reserves here and lets
        // context_allocate consume the same slice.
        self.rsx_context.pending_mem_addr = addr;

        let handle_write = Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(mem_handle_ptr as u64), 4).unwrap(),
            bytes: WritePayload::from_slice(&handle.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };
        let addr_write = Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(mem_addr_ptr as u64), 8).unwrap(),
            bytes: WritePayload::from_slice(&(addr as u64).to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };

        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![handle_write, addr_write],
        }
    }

    /// sys_rsx_memory_free (667). No-op: the bump allocator never
    /// reclaims. Returns CELL_OK matching the `sys_memory_free`
    /// stub; titles that key on errno will misbehave.
    pub(super) fn dispatch_sys_rsx_memory_free_noop(&self) -> Lv2Dispatch {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    }

    /// sys_rsx_context_allocate (670). Allocates a 0x300000-byte
    /// reservation out of the sys_rsx region, splits it into
    /// DMA-control / driver-info / reports sub-regions at the LV2
    /// fixed offsets, writes the four out-pointers + `context_id`,
    /// seeds the init patterns into reports and driver-info, and
    /// creates the handler event-queue / port pair.
    ///
    /// Returns `CELL_EINVAL` on double-allocate (single-context
    /// invariant) and `CELL_ENOMEM` if the 0x300000-byte reservation
    /// does not fit in the remaining sys_rsx region.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn dispatch_sys_rsx_context_allocate(
        &mut self,
        context_id_ptr: u32,
        lpar_dma_control_ptr: u32,
        lpar_driver_info_ptr: u32,
        lpar_reports_ptr: u32,
        mem_ctx: u64,
        system_mode: u64,
        requester: UnitId,
    ) -> Lv2Dispatch {
        if self.rsx_context.allocated {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_EINVAL.into(),
                effects: vec![],
            };
        }
        // If a prior sys_rsx_memory_allocate reserved the slice,
        // reuse its base. Otherwise carve out a fresh 0x300000
        // region at the cursor and advance.
        let base = if self.rsx_context.pending_mem_addr != 0 {
            self.rsx_context.pending_mem_addr
        } else {
            let Some(end) = self.rsx_mem_alloc_ptr.checked_add(RSX_CONTEXT_RESERVATION) else {
                return Lv2Dispatch::Immediate {
                    code: crate::errno::CELL_ENOMEM.into(),
                    effects: vec![],
                };
            };
            if end > Self::SYS_RSX_MEM_END {
                return Lv2Dispatch::Immediate {
                    code: crate::errno::CELL_ENOMEM.into(),
                    effects: vec![],
                };
            }
            let start = self.rsx_mem_alloc_ptr;
            self.rsx_mem_alloc_ptr = end;
            start
        };
        let dma_control_addr = base + DMA_CONTROL_OFFSET;
        let driver_info_addr = base + DRIVER_INFO_OFFSET;
        let reports_addr = base + REPORTS_OFFSET;

        // LV2 creates a local event port + queue pair and writes
        // the queue handle (overwriting the port handle) into
        // driver_info.handler_queue. CellGov's event model uses
        // port_id == queue_id as a 1:1 binding, so a single kernel
        // id serves both roles.
        let queue_id = self.alloc_id();
        let queue_created = self
            .event_queues
            .create_with_id(queue_id, RSX_EVENT_QUEUE_SIZE);
        debug_assert!(
            queue_created,
            "sys_rsx event queue id {queue_id:#x} collided with existing queue"
        );

        // Preserve the pending_mem_addr (and any future carry-over
        // state) set by sys_rsx_memory_allocate; context_allocate
        // only claims the slice, it does not clear the tracking.
        self.rsx_context = SysRsxContext {
            allocated: true,
            context_id: RSX_CONTEXT_ID,
            dma_control_addr,
            driver_info_addr,
            reports_addr,
            event_queue_id: queue_id,
            event_port_id: queue_id,
            mem_ctx,
            system_mode,
            pending_mem_addr: self.rsx_context.pending_mem_addr,
            ..SysRsxContext::new()
        };

        let mk_write_u32 = |ptr: u32, value: u32| Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(ptr as u64), 4).unwrap(),
            bytes: WritePayload::from_slice(&value.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };
        let mk_write_u64 = |ptr: u32, value: u32| Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(ptr as u64), 8).unwrap(),
            bytes: WritePayload::from_slice(&(value as u64).to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };

        let mut reports_bytes = vec![0u8; RSX_REPORTS_SIZE];
        write_rsx_reports_init(&mut reports_bytes);
        let reports_init = Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(reports_addr as u64), RSX_REPORTS_SIZE as u64)
                .unwrap(),
            bytes: WritePayload::from_slice(&reports_bytes),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };

        let mut driver_info_bytes = vec![0u8; RSX_DRIVER_INFO_SIZE];
        write_rsx_driver_info_init(
            &mut driver_info_bytes,
            driver_info_init::MEMORY_SIZE,
            system_mode as u32,
            queue_id,
        );
        let driver_info_init_effect = Effect::SharedWriteIntent {
            range: ByteRange::new(
                GuestAddr::new(driver_info_addr as u64),
                RSX_DRIVER_INFO_SIZE as u64,
            )
            .unwrap(),
            bytes: WritePayload::from_slice(&driver_info_bytes),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };

        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![
                mk_write_u32(context_id_ptr, RSX_CONTEXT_ID),
                mk_write_u64(lpar_dma_control_ptr, dma_control_addr),
                mk_write_u64(lpar_driver_info_ptr, driver_info_addr),
                mk_write_u64(lpar_reports_ptr, reports_addr),
                reports_init,
                driver_info_init_effect,
            ],
        }
    }

    /// sys_rsx_context_free (671). No-op: the single-context model
    /// does not tear down state. Returns CELL_OK; a subsequent
    /// `sys_rsx_context_allocate` would still be rejected by the
    /// `allocated` flag.
    pub(super) fn dispatch_sys_rsx_context_free_noop(&self) -> Lv2Dispatch {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    }

    /// sys_rsx_context_attribute (674). Dispatches on `package_id`.
    /// Each sub-command lands in a later slice; the scaffold returns
    /// CELL_OK for unknown / not-yet-wired sub-commands and logs one
    /// invariant-break entry so the first unhandled package_id is
    /// visible without silent success.
    pub(super) fn dispatch_sys_rsx_context_attribute(
        &mut self,
        context_id: u32,
        package_id: u32,
        _a3: u64,
        _a4: u64,
        _a5: u64,
        _a6: u64,
    ) -> Lv2Dispatch {
        if !self.rsx_context.allocated || context_id != self.rsx_context.context_id {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_EINVAL.into(),
                effects: vec![],
            };
        }
        match package_id {
            PACKAGE_FLIP_MODE => {
                self.rsx_context.flip_mode = _a4 as u32;
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![],
                }
            }
            PACKAGE_FLIP_BUFFER => self.sys_rsx_attribute_flip(_a3, _a4),
            PACKAGE_SET_DISPLAY_BUFFER => self.sys_rsx_attribute_set_display_buffer(_a3, _a4, _a5),
            PACKAGE_CELLGOV_SET_FLIP_HANDLER => {
                self.rsx_context.flip_handler_addr = _a3 as u32;
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![],
                }
            }
            PACKAGE_CELLGOV_SET_VBLANK_HANDLER => {
                self.rsx_context.vblank_handler_addr = _a3 as u32;
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![],
                }
            }
            PACKAGE_CELLGOV_SET_USER_HANDLER => {
                self.rsx_context.user_handler_addr = _a3 as u32;
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![],
                }
            }
            _ => self.sys_rsx_attribute_unknown(package_id),
        }
    }

    /// 0x104 SET_DISPLAY_BUFFER: decodes and records a slot per
    /// RPCS3 `sys_rsx.cpp:624-647`. `display_buffers_count` rises
    /// monotonically to `id + 1`.
    fn sys_rsx_attribute_set_display_buffer(&mut self, a3: u64, a4: u64, a5: u64) -> Lv2Dispatch {
        let id = (a3 & 0xFF) as usize;
        if id >= DISPLAY_BUFFER_COUNT {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_EINVAL.into(),
                effects: vec![],
            };
        }
        let width = (a4 >> 32) as u32;
        let height = a4 as u32;
        let pitch = (a5 >> 32) as u32;
        let offset = a5 as u32;
        self.rsx_context.display_buffers[id] = RsxDisplayBuffer {
            offset,
            pitch,
            width,
            height,
        };
        let next_count = (id as u32) + 1;
        if next_count > self.rsx_context.display_buffers_count {
            self.rsx_context.display_buffers_count = next_count;
        }
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    }

    /// 0x102 FLIP_BUFFER: emits an [`Effect::RsxFlipRequest`] so the
    /// commit pipeline drives WAITING -> DONE on the flip-status
    /// state machine. `a3` is the head index; `a4` is the flip
    /// target encoding. Observable transition matches
    /// NV4097_FLIP_BUFFER.
    fn sys_rsx_attribute_flip(&self, _head: u64, flip_target: u64) -> Lv2Dispatch {
        // Queued-path high bit: low 4 bits hold the buffer index
        // (RPCS3 sys_rsx.cpp:562). Direct-path: we don't track
        // display_buffers here, so record 0 as the index. The flip-
        // status state machine only keys on pending/done transitions,
        // not the recorded index.
        let buffer_index: u8 = if (flip_target & 0x8000_0000) != 0 {
            (flip_target & 0x0F) as u8
        } else {
            0
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![Effect::RsxFlipRequest { buffer_index }],
        }
    }

    fn sys_rsx_attribute_unknown(&mut self, package_id: u32) -> Lv2Dispatch {
        self.log_invariant_break(
            "dispatch.sys_rsx_context_attribute_unknown_package",
            format_args!(
                "sys_rsx_context_attribute package_id {package_id:#x} not yet wired; \
                 returning CELL_OK stub"
            ),
        );
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::test_support::{extract_write_u32, FakeRuntime};
    use crate::request::Lv2Request;

    fn extract_write_u64(effect: &Effect) -> u64 {
        let Effect::SharedWriteIntent { bytes, .. } = effect else {
            panic!("expected SharedWriteIntent, got {effect:?}");
        };
        let b = bytes.bytes();
        assert_eq!(b.len(), 8);
        u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    }

    fn allocate_rsx(host: &mut Lv2Host, size: u32, source: UnitId) -> (u32, u64) {
        let rt = FakeRuntime::new(0x10_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxMemoryAllocate {
                mem_handle_ptr: 0x1000,
                mem_addr_ptr: 0x2000,
                size,
                flags: 0,
                a5: 0,
                a6: 0,
                a7: 0,
            },
            source,
            &rt,
        );
        match d {
            Lv2Dispatch::Immediate { code: 0, effects } => (
                extract_write_u32(&effects[0]),
                extract_write_u64(&effects[1]),
            ),
            other => panic!("expected Immediate(0), got {other:?}"),
        }
    }

    #[test]
    fn sys_rsx_memory_allocate_returns_base_then_bumps() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);

        let (h1, a1) = allocate_rsx(&mut host, 0x30_0000, source);
        assert_eq!(h1, 1);
        assert_eq!(a1, Lv2Host::SYS_RSX_MEM_BASE as u64);

        let (h2, a2) = allocate_rsx(&mut host, 0x30_0000, source);
        assert_eq!(h2, 2);
        assert_eq!(a2, (Lv2Host::SYS_RSX_MEM_BASE + 0x30_0000) as u64);
    }

    #[test]
    fn sys_rsx_memory_allocate_rejects_zero_size() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxMemoryAllocate {
                mem_handle_ptr: 0x1000,
                mem_addr_ptr: 0x2000,
                size: 0,
                flags: 0,
                a5: 0,
                a6: 0,
                a7: 0,
            },
            UnitId::new(0),
            &rt,
        );
        assert!(matches!(
            d,
            Lv2Dispatch::Immediate { code, .. } if code == u64::from(crate::errno::CELL_ENOMEM)
        ));
    }

    fn context_allocate_request(
        context_id_ptr: u32,
        lpar_dma_control_ptr: u32,
        lpar_driver_info_ptr: u32,
        lpar_reports_ptr: u32,
        mem_ctx: u64,
    ) -> Lv2Request {
        Lv2Request::SysRsxContextAllocate {
            context_id_ptr,
            lpar_dma_control_ptr,
            lpar_driver_info_ptr,
            lpar_reports_ptr,
            mem_ctx,
            system_mode: 0,
        }
    }

    #[test]
    fn sys_rsx_context_allocate_writes_four_out_pointers_and_reports_init() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x1_0000);
        let source = UnitId::new(0);

        let d = host.dispatch(
            context_allocate_request(0x1000, 0x1008, 0x1010, 0x1018, 0xA001),
            source,
            &rt,
        );
        let Lv2Dispatch::Immediate { code: 0, effects } = d else {
            panic!("expected Immediate(0), got {d:?}");
        };
        assert_eq!(effects.len(), 6);
        assert_eq!(extract_write_u32(&effects[0]), RSX_CONTEXT_ID);
        assert_eq!(
            extract_write_u64(&effects[1]),
            Lv2Host::SYS_RSX_MEM_BASE as u64
        );
        assert_eq!(
            extract_write_u64(&effects[2]),
            (Lv2Host::SYS_RSX_MEM_BASE + DRIVER_INFO_OFFSET) as u64
        );
        assert_eq!(
            extract_write_u64(&effects[3]),
            (Lv2Host::SYS_RSX_MEM_BASE + REPORTS_OFFSET) as u64
        );

        // Fifth effect: 37 KB init targeting reports_addr.
        // Sanity-check the LV2 sentinel at label 255 (offset 0xFF0).
        let Effect::SharedWriteIntent { range, bytes, .. } = &effects[4] else {
            panic!("expected SharedWriteIntent for reports init");
        };
        assert_eq!(
            range.start().raw(),
            (Lv2Host::SYS_RSX_MEM_BASE + REPORTS_OFFSET) as u64
        );
        let b = bytes.bytes();
        assert_eq!(b.len(), RSX_REPORTS_SIZE);
        let sentinel = u32::from_be_bytes([b[0xFF0], b[0xFF1], b[0xFF2], b[0xFF3]]);
        assert_eq!(sentinel, 0x1337_C0D3);
        assert_eq!(&b[0x1000..0x1008], &[0xFF; 8]);
        assert_eq!(&b[0x140C..0x1410], &[0xFF; 4]);

        // Sixth effect: 0x12F8-byte driver-info init at driver_info_addr.
        let Effect::SharedWriteIntent { range, bytes, .. } = &effects[5] else {
            panic!("expected SharedWriteIntent for driver-info init");
        };
        assert_eq!(
            range.start().raw(),
            (Lv2Host::SYS_RSX_MEM_BASE + DRIVER_INFO_OFFSET) as u64
        );
        let b = bytes.bytes();
        assert_eq!(b.len(), RSX_DRIVER_INFO_SIZE);
        assert_eq!(
            u32::from_be_bytes([b[0x00], b[0x01], b[0x02], b[0x03]]),
            driver_info_init::VERSION_DRIVER
        );
        assert_eq!(
            u32::from_be_bytes([b[0x04], b[0x05], b[0x06], b[0x07]]),
            driver_info_init::VERSION_GPU
        );
        assert_eq!(
            u32::from_be_bytes([b[0x0C], b[0x0D], b[0x0E], b[0x0F]]),
            driver_info_init::HARDWARE_CHANNEL
        );
        assert_eq!(
            u32::from_be_bytes([b[0x10], b[0x11], b[0x12], b[0x13]]),
            driver_info_init::NVCORE_FREQUENCY
        );
        assert_eq!(
            u32::from_be_bytes([b[0x2C], b[0x2D], b[0x2E], b[0x2F]]),
            driver_info_init::REPORTS_NOTIFY_OFFSET
        );
        assert_eq!(
            u32::from_be_bytes([b[0x34], b[0x35], b[0x36], b[0x37]]),
            driver_info_init::REPORTS_REPORT_OFFSET
        );

        let ctx = host.sys_rsx_context();
        assert!(ctx.allocated);
        assert_eq!(ctx.context_id, RSX_CONTEXT_ID);
        assert_eq!(ctx.dma_control_addr, Lv2Host::SYS_RSX_MEM_BASE);
        assert_eq!(
            ctx.driver_info_addr,
            Lv2Host::SYS_RSX_MEM_BASE + DRIVER_INFO_OFFSET
        );
        assert_eq!(ctx.reports_addr, Lv2Host::SYS_RSX_MEM_BASE + REPORTS_OFFSET);
        assert_eq!(ctx.mem_ctx, 0xA001);
    }

    #[test]
    fn write_rsx_reports_init_matches_rpcs3_pattern() {
        let mut expected = vec![0u8; RSX_REPORTS_SIZE];
        for i in 0..1024 {
            let offset = i * 4;
            expected[offset..offset + 4]
                .copy_from_slice(&SEMAPHORE_INIT_PATTERN[i % 4].to_be_bytes());
        }
        for i in 0..64 {
            let offset = 0x1000 + i * 16;
            expected[offset..offset + 8].copy_from_slice(&u64::MAX.to_be_bytes());
        }
        for i in 0..2048 {
            let offset = 0x1400 + i * 16;
            expected[offset..offset + 8].copy_from_slice(&u64::MAX.to_be_bytes());
            expected[offset + 12..offset + 16].copy_from_slice(&u32::MAX.to_be_bytes());
        }
        let mut actual = vec![0u8; RSX_REPORTS_SIZE];
        write_rsx_reports_init(&mut actual);
        assert_eq!(actual, expected);
    }

    #[test]
    #[should_panic(expected = "RSX_REPORTS_SIZE-byte buffer")]
    fn write_rsx_reports_init_rejects_wrong_size() {
        let mut buf = vec![0u8; 128];
        write_rsx_reports_init(&mut buf);
    }

    #[test]
    fn write_rsx_driver_info_init_stamps_all_fields() {
        let mut buf = vec![0u8; RSX_DRIVER_INFO_SIZE];
        write_rsx_driver_info_init(&mut buf, 0x0F90_0000, 0xABCD, 0xE001);
        let read = |o: usize| u32::from_be_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]);
        assert_eq!(read(0x00), driver_info_init::VERSION_DRIVER);
        assert_eq!(read(0x04), driver_info_init::VERSION_GPU);
        assert_eq!(read(0x08), 0x0F90_0000);
        assert_eq!(read(0x0C), driver_info_init::HARDWARE_CHANNEL);
        assert_eq!(read(0x10), driver_info_init::NVCORE_FREQUENCY);
        assert_eq!(read(0x14), driver_info_init::MEMORY_FREQUENCY);
        assert_eq!(read(0x2C), driver_info_init::REPORTS_NOTIFY_OFFSET);
        assert_eq!(read(0x30), driver_info_init::REPORTS_OFFSET_FIELD);
        assert_eq!(read(0x34), driver_info_init::REPORTS_REPORT_OFFSET);
        assert_eq!(read(0x50), 0xABCD);
        assert_eq!(read(DRIVER_INFO_HANDLER_QUEUE_OFFSET), 0xE001);
    }

    #[test]
    #[should_panic(expected = "RSX_DRIVER_INFO_SIZE-byte buffer")]
    fn write_rsx_driver_info_init_rejects_wrong_size() {
        let mut buf = vec![0u8; 128];
        write_rsx_driver_info_init(&mut buf, 0, 0, 0);
    }

    fn allocate_context(host: &mut Lv2Host, source: UnitId) {
        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            context_allocate_request(0x1000, 0x1008, 0x1010, 0x1018, 0xA001),
            source,
            &rt,
        );
        assert!(matches!(d, Lv2Dispatch::Immediate { code: 0, .. }));
    }

    #[test]
    fn sys_rsx_context_attribute_flip_emits_flip_request() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        // Queued path: high bit set, low nibble = 3.
        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: PACKAGE_FLIP_BUFFER,
                a3: 0,
                a4: 0x8000_0003,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        let Lv2Dispatch::Immediate { code: 0, effects } = d else {
            panic!("expected Immediate(0), got {d:?}");
        };
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::RsxFlipRequest { buffer_index: 3 }
        ));
    }

    #[test]
    fn sys_rsx_context_attribute_flip_direct_path_uses_zero_index() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: PACKAGE_FLIP_BUFFER,
                a3: 0,
                a4: 0x0000_1234,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        let Lv2Dispatch::Immediate { code: 0, effects } = d else {
            panic!("expected Immediate(0), got {d:?}");
        };
        assert!(matches!(
            effects[0],
            Effect::RsxFlipRequest { buffer_index: 0 }
        ));
    }

    #[test]
    fn sys_rsx_context_attribute_set_flip_handler_records_callback() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: PACKAGE_CELLGOV_SET_FLIP_HANDLER,
                a3: 0xDEAD_BEEF,
                a4: 0,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        assert!(matches!(d, Lv2Dispatch::Immediate { code: 0, .. }));
        assert_eq!(host.sys_rsx_context().flip_handler_addr, 0xDEAD_BEEF);
        assert_eq!(host.sys_rsx_context().vblank_handler_addr, 0);
        assert_eq!(host.sys_rsx_context().user_handler_addr, 0);
    }

    #[test]
    fn sys_rsx_context_attribute_set_vblank_handler_records_callback() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: PACKAGE_CELLGOV_SET_VBLANK_HANDLER,
                a3: 0xCAFE_F00D,
                a4: 0,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        assert_eq!(host.sys_rsx_context().vblank_handler_addr, 0xCAFE_F00D);
    }

    #[test]
    fn sys_rsx_context_attribute_set_user_handler_records_callback() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: PACKAGE_CELLGOV_SET_USER_HANDLER,
                a3: 0xABCD_0001,
                a4: 0,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        assert_eq!(host.sys_rsx_context().user_handler_addr, 0xABCD_0001);
    }

    #[test]
    fn sys_rsx_context_attribute_null_flip_handler_clears() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);
        let rt = FakeRuntime::new(0x1_0000);
        host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: PACKAGE_CELLGOV_SET_FLIP_HANDLER,
                a3: 0x1234_5678,
                a4: 0,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: PACKAGE_CELLGOV_SET_FLIP_HANDLER,
                a3: 0,
                a4: 0,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        assert_eq!(host.sys_rsx_context().flip_handler_addr, 0);
    }

    #[test]
    fn sys_rsx_context_attribute_flip_mode_records_mode() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: PACKAGE_FLIP_MODE,
                a3: 0,
                a4: 2, // vsync
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        assert_eq!(host.sys_rsx_context().flip_mode, 2);
    }

    #[test]
    fn sys_rsx_context_attribute_set_display_buffer_records_slot() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        // Slot 1: width=1920, height=1080, pitch=0x2000, offset=0x10_0000
        host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: PACKAGE_SET_DISPLAY_BUFFER,
                a3: 1,
                a4: (1920u64 << 32) | 1080,
                a5: (0x2000u64 << 32) | 0x10_0000,
                a6: 0,
            },
            source,
            &rt,
        );
        let ctx = host.sys_rsx_context();
        assert_eq!(ctx.display_buffers_count, 2);
        let slot = ctx.display_buffers[1];
        assert_eq!(slot.width, 1920);
        assert_eq!(slot.height, 1080);
        assert_eq!(slot.pitch, 0x2000);
        assert_eq!(slot.offset, 0x10_0000);
    }

    #[test]
    fn sys_rsx_context_attribute_set_display_buffer_rejects_id_over_7() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: PACKAGE_SET_DISPLAY_BUFFER,
                a3: 8, // invalid
                a4: 0,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        assert!(matches!(
            d,
            Lv2Dispatch::Immediate { code, .. } if code == u64::from(crate::errno::CELL_EINVAL)
        ));
    }

    #[test]
    fn sys_rsx_context_attribute_unknown_package_returns_ok() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: 0xBEEF,
                a3: 0,
                a4: 0,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        assert!(matches!(
            d,
            Lv2Dispatch::Immediate { code: 0, effects } if effects.is_empty()
        ));
    }

    #[test]
    fn sys_rsx_context_attribute_rejects_unallocated_context() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: 0x102,
                a3: 0,
                a4: 0,
                a5: 0,
                a6: 0,
            },
            UnitId::new(0),
            &rt,
        );
        assert!(matches!(
            d,
            Lv2Dispatch::Immediate { code, .. } if code == u64::from(crate::errno::CELL_EINVAL)
        ));
    }

    #[test]
    fn sys_rsx_context_attribute_rejects_wrong_context_id() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: 0xDEAD_BEEF,
                package_id: 0x102,
                a3: 0,
                a4: 0,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        assert!(matches!(
            d,
            Lv2Dispatch::Immediate { code, .. } if code == u64::from(crate::errno::CELL_EINVAL)
        ));
    }

    #[test]
    fn sys_rsx_memory_free_returns_ok() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxMemoryFree { mem_handle: 0xA001 },
            UnitId::new(0),
            &rt,
        );
        assert!(matches!(
            d,
            Lv2Dispatch::Immediate { code: 0, effects } if effects.is_empty()
        ));
    }

    #[test]
    fn sys_rsx_context_free_returns_ok() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxContextFree {
                context_id: RSX_CONTEXT_ID,
            },
            UnitId::new(0),
            &rt,
        );
        assert!(matches!(
            d,
            Lv2Dispatch::Immediate { code: 0, effects } if effects.is_empty()
        ));
    }

    #[test]
    fn sys_rsx_context_allocate_registers_event_queue_in_handler_queue() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x1_0000);
        let source = UnitId::new(0);

        let d = host.dispatch(
            context_allocate_request(0x1000, 0x1008, 0x1010, 0x1018, 0xA001),
            source,
            &rt,
        );
        let Lv2Dispatch::Immediate { code: 0, effects } = d else {
            panic!("expected Immediate(0), got {d:?}");
        };
        let Effect::SharedWriteIntent { bytes, .. } = &effects[5] else {
            panic!("expected SharedWriteIntent for driver-info init");
        };
        let b = bytes.bytes();
        let queue_id = u32::from_be_bytes([
            b[DRIVER_INFO_HANDLER_QUEUE_OFFSET],
            b[DRIVER_INFO_HANDLER_QUEUE_OFFSET + 1],
            b[DRIVER_INFO_HANDLER_QUEUE_OFFSET + 2],
            b[DRIVER_INFO_HANDLER_QUEUE_OFFSET + 3],
        ]);
        assert_ne!(queue_id, 0);
        let ctx = host.sys_rsx_context();
        assert_eq!(ctx.event_queue_id, queue_id);
        assert_eq!(ctx.event_port_id, queue_id);
    }

    #[test]
    fn sys_rsx_context_allocate_second_call_rejects_with_einval() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x1_0000);
        let source = UnitId::new(0);

        let _ = host.dispatch(
            context_allocate_request(0x1000, 0x1008, 0x1010, 0x1018, 0xA001),
            source,
            &rt,
        );
        let d = host.dispatch(
            context_allocate_request(0x2000, 0x2008, 0x2010, 0x2018, 0xA001),
            source,
            &rt,
        );
        assert!(matches!(
            d,
            Lv2Dispatch::Immediate { code, effects } if code == u64::from(crate::errno::CELL_EINVAL) && effects.is_empty()
        ));
    }

    #[test]
    fn sys_rsx_memory_allocate_rejects_beyond_region_end() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxMemoryAllocate {
                mem_handle_ptr: 0x1000,
                mem_addr_ptr: 0x2000,
                size: 0x2000_0000,
                flags: 0,
                a5: 0,
                a6: 0,
                a7: 0,
            },
            UnitId::new(0),
            &rt,
        );
        assert!(matches!(
            d,
            Lv2Dispatch::Immediate { code, .. } if code == u64::from(crate::errno::CELL_ENOMEM)
        ));
    }
}
