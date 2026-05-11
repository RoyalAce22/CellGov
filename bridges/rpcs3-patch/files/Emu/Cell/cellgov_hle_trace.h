// CellGov HLE-trace hook. Env-var-gated capture of every BIND_FUNC
// HLE call entry/exit, with optional watch-address diff so an
// investigator can ask "which NID wrote this guest address?" in one
// run instead of N hours of Ghidra per candidate.
//
// Environment variables:
//
//   CELLGOV_HLE_TRACE_PATH   Path to output binary trace file. When
//                            unset the hook is a no-op. The file is
//                            overwritten on each run.
//
//   CELLGOV_HLE_WATCH        Optional comma-separated list of
//                            `addr:size` pairs in hex (e.g.
//                            0x101e3cb8:8,0x101e3ca0:32). Each
//                            region is snapshotted at every HLE
//                            entry and diffed at exit; bytes that
//                            changed are recorded on the call's
//                            record. Empty list -> records carry no
//                            writes (still useful for raw call
//                            traces).
//
// The CellGov side consumes the trace via
// `cellgov_cli rpcs3-attribute --trace <file> --addr 0xADDR`,
// which prints the HLE call(s) whose write set covers `addr`.
//
// File format (little-endian, no padding between records):
//
//   [Header]
//     u32 magic = 0xC0E60001
//     u32 version = 2
//
//   [Record], repeated:
//     u32 record_magic = 0xC0E60002
//     u64 step           // PPU CIA at HLE entry (the `sc` PC for syscalls,
//                        // or the BIND_FUNC dispatch PC for module funcs).
//                        // NOT a timestamp: records are emitted in file
//                        // order which is chronological.
//     u64 lr             // PPU LR at HLE entry. For HLE module functions
//                        // this is the user-code call site; for syscalls
//                        // it is the wrapper return address. Combined with
//                        // `step` lets investigators triangulate the user
//                        // code path between two HLE calls.
//     u32 thread_id      // PPU thread id
//     u32 depth          // call depth (0 = outermost HLE call)
//     u32 name_len
//     bytes[name_len] name
//     u64[8] args        // r3..r10 at entry
//     u64 ret            // r3 at exit
//     u32 num_writes
//     [Write], repeated num_writes times:
//       u64 addr
//       u32 size
//       bytes[size] new_bytes
//
// Synthetic `<guest_code>` records emitted by between-call drift
// detection use `step = cur_entry_pc`, `lr = prev_exit_lr`,
// `args[0] = prev_exit_pc`, `args[1] = cur_entry_pc`.

#pragma once

#include "util/types.hpp"

class ppu_thread;

namespace cellgov_hle_trace
{
	// Called once on first BIND_FUNC invocation per process. Cheap
	// no-op after the first call. Reads CELLGOV_HLE_TRACE_PATH and
	// CELLGOV_HLE_WATCH; opens the trace file if a path is set.
	void ensure_initialized();

	// True iff CELLGOV_HLE_TRACE_PATH was set when ensure_initialized
	// ran. Used by BIND_FUNC to short-circuit the hook in the
	// dominant non-tracing case without paying the env-var lookup
	// per call.
	bool is_active();

	// Push a new call frame onto the current thread's HLE stack.
	// `name` is the C string baked in at BIND_FUNC site (lives for
	// the process lifetime). `args` is the 8-element r3..r10 view
	// already captured into ppu.syscall_args by BIND_FUNC.
	void enter(ppu_thread& ppu, const char* name, const u64* args);

	// Pop the current thread's top frame and emit a record. Reads
	// the diff of any watch addresses against the snapshot taken in
	// `enter`. `ret` is r3 at exit.
	void exit(ppu_thread& ppu, u64 ret);
}
