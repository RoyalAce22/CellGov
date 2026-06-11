// CellGov PPU instruction trace hook (Stage 40D.2). Env-var-gated
// capture of (pre_state, instruction, post_state, mem_pre, mem_post)
// tuples for every PPU instruction in a configurable filter window.
// The CellGov side ingests the dump and replays each record through
// its decoder + executor, asserting bit-equality against the captured
// post-state -- the per-instruction differential against RPCS3.
//
// Environment variables:
//
//   CELLGOV_PPU_TRACE_PATH        Path to output binary trace file. When
//                                 unset the hook is a no-op. The file is
//                                 overwritten on each run.
//
//   CELLGOV_PPU_TRACE_PC_RANGE    Optional `start-end` (hex) PC range
//                                 (inclusive on both ends). When set,
//                                 only instructions whose PC falls in
//                                 the range are emitted. Example:
//                                 0x00100000-0x00200000.
//
//   CELLGOV_PPU_TRACE_PRIMARY     Optional comma-separated primary-opcode
//                                 list (decimal). When set, only
//                                 instructions whose top 6 bits match
//                                 are emitted. Example: 31,30,46.
//
//   CELLGOV_PPU_TRACE_MAX_RECORDS Optional decimal cap on the number of
//                                 records emitted. Default unlimited.
//
//   CELLGOV_PPU_TRACE_MAX_BYTES   Optional decimal cap on the trace file
//                                 size in bytes. Default unlimited.
//
// Filters are AND-combined; a record is emitted only when every active
// filter matches.
//
// File format (little-endian, no padding between records):
//
//   [Header]
//     u32 magic = 0xC0E60003
//     u32 version = 1
//
//   [Record], repeated:
//     u32 record_magic = 0xC0E60004
//     u64 pc
//     u32 raw_instruction              // big-endian-on-disk PPC word
//                                      // (matches guest-memory storage)
//     u32 thread_id
//     u64[32] pre_gpr                  // GPRs at entry
//     u64[32] pre_fpr                  // FPR raw bit patterns
//     u8[16][32] pre_vr                // VRs, 16 bytes each (byte 0 MSB)
//     u32 pre_cr
//     u64 pre_lr
//     u64 pre_ctr
//     u64 pre_xer
//     u32 pre_raddr                    // reservation address before
//                                      // the instruction (0 means no
//                                      // active reservation)
//     u64 pre_rtime                    // reservation acquire timestamp
//                                      // (RPCS3 ppu_thread::rtime).
//                                      // Diagnostic-only on the
//                                      // CellGov side.
//     u64[32] post_gpr
//     u64[32] post_fpr
//     u8[16][32] post_vr
//     u32 post_cr
//     u64 post_lr
//     u64 post_ctr
//     u64 post_xer
//     u32 post_raddr                   // reservation address after
//     u64 post_rtime                   // reservation timestamp after
//     u64 mem_addr                     // start of touched memory window
//                                      // (0 if mem_len == 0)
//     u32 mem_len                      // length in bytes (0 if no memory
//                                      // accessed)
//     u8[mem_len] mem_pre              // memory contents BEFORE the
//                                      // instruction
//     u8[mem_len] mem_post             // memory contents AFTER the
//                                      // instruction
//
// The memory window must contain every byte the instruction read or
// wrote. For loads, `mem_pre == mem_post` and carries the bytes the
// load consumed. For stores, the diff between `mem_pre` and `mem_post`
// is the store payload. For non-memory instructions, `mem_len == 0`
// and no bytes follow.
//
// Read-only with respect to guest state: the hook reads ppu_thread
// registers and vm:: bytes, never mutates. Lives entirely outside the
// emulated guest's address space.

#pragma once

#include "util/types.hpp"

class ppu_thread;

namespace cellgov_ppu_trace
{
	// File-format magic constants. Version 2 added a per-state
	// reservation address (u32, RPCS3's `ppu_thread::raddr`) after
	// xer so the harness can replay lwarx / ldarx / stwcx / stdcx
	// sequences with the reservation state restored. Version 3
	// adds the reservation acquire timestamp (u64,
	// `ppu_thread::rtime`) after raddr -- diagnostic-only on the
	// CellGov side (no rtime equivalent) but lets the harness
	// explain why a captured stwcx fails: RPCS3 invalidates a
	// reservation when the reservation table's per-line timestamp
	// no longer matches rtime, and that invalidation is invisible
	// to single-instruction replay.
	constexpr u32 HEADER_MAGIC = 0xC0E60003u;
	constexpr u32 RECORD_MAGIC = 0xC0E60004u;
	constexpr u32 FORMAT_VERSION = 3u;

	// Called once per process on first instruction dispatch. Reads the
	// CELLGOV_PPU_TRACE_* env vars, opens the trace file if a path is
	// set, parses filter lists. Cheap no-op after the first call.
	void ensure_initialized();

	// True iff CELLGOV_PPU_TRACE_PATH was set when ensure_initialized
	// ran. Used by the PPU dispatch hook to short-circuit the trace
	// machinery in the dominant non-tracing case.
	bool is_active();

	// Per-instruction hook. Called with the pre-execution PPU state.
	// The implementation:
	//   1. Classifies the raw instruction into a memory access shape
	//      (`addr`, `len`) via an internal opcode lookup table; ALU /
	//      branch / VRT-only instructions get `len = 0`.
	//   2. Captures pre-state (registers + the `mem_pre` byte window
	//      if non-empty) into thread-local storage.
	//   3. Arranges for `emit_record` to consume the matching
	//      thread-local data after dispatch returns.
	//
	// Implementations may bail early when any active filter rejects
	// the instruction; in that case the matching `emit_record` call
	// is a no-op.
	void on_pre_dispatch(ppu_thread& ppu, u64 pc, u32 raw);

	// Companion to `on_pre_dispatch`. Reads the post-state register
	// file and the post-execution `mem_post` byte window the
	// pre-dispatch classifier scoped, then writes one record to the
	// trace file.
	void emit_record(ppu_thread& ppu);
}
