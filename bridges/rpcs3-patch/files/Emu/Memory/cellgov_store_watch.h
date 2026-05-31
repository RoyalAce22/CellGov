// CellGov store-watch hook (patch 0003 surface). Per-store
// instruction-level watch on a configured guest EA range. Every PPU
// store landing in [addr, addr+len) is recorded as
// (step, pc, ea, width, value) to a host file.
//
// Env vars:
//
//   CELLGOV_STORE_WATCH       addr:len hex pair (e.g. 0x91fe90:0x20).
//   CELLGOV_STORE_WATCH_PATH  host file to write the log to.
//                             Overwritten on each run.
//
// File format (little-endian):
//
//   header  bytes 0x00..0x10: 'C','G','S','W', version u32 = 1,
//                             watch_addr u32, watch_len u32
//   record  28 bytes: { step u64, pc u32, ea u32, width u32,
//                       value u64 }
//
// Coverage caveat: this hook fires from vm::write_u8/16/32/64. The
// PPU LLVM recompiler bypasses these in favor of inline LLVM store
// IR, so JIT-mode runs capture only the subset RPCS3 routes through
// vm::write* (MMIO-suspected writes via `__write_maybe_mmio32`,
// syscalls, HLE thunks). For complete per-store coverage of guest
// data writes, run with Core::PPU_Decoder = Interpreter for the
// watch capture. PPU semantics are deterministic, so interpreter
// and JIT produce identical store sequences for the same code path;
// the per-store PC the watch records is the same.
//
// Read-only with respect to guest state. Cost on the non-watching
// path: one std::call_once check (already-initialized fast path)
// plus one atomic-load per vm::write* call.
//
// This header is intentionally vm.h-free so vm.h can include it
// without a circular dependency.

#pragma once

#include "util/types.hpp"

#include <atomic>
#include <cstdio>
#include <cstdlib>
#include <mutex>
#include <string>

namespace cellgov
{
	struct store_watch_state
	{
		std::atomic<bool> active{false};
		u32 watch_addr = 0;
		u32 watch_len = 0;
		std::FILE* file = nullptr;
		std::atomic<u64> step{0};
		std::mutex write_mu;
	};

	// C++17 inline variable: single shared instance across TUs that
	// include this header. The vm.h hook sees the same state as any
	// other consumer.
	inline store_watch_state g_store_watch{};

	inline void cellgov_store_watch_init_once()
	{
		static std::once_flag init_flag;
		std::call_once(init_flag, []()
		{
			const char* spec = std::getenv("CELLGOV_STORE_WATCH");
			const char* path = std::getenv("CELLGOV_STORE_WATCH_PATH");
			if (!spec || !*spec || !path || !*path) return;

			std::string ss(spec);
			std::size_t colon = ss.find(':');
			if (colon == std::string::npos)
			{
				std::fprintf(stderr, "[cellgov] CELLGOV_STORE_WATCH missing ':' (expected addr:len): %s\n", spec);
				return;
			}
			const unsigned long long addr_v = std::strtoull(ss.substr(0, colon).c_str(), nullptr, 16);
			const unsigned long long len_v  = std::strtoull(ss.substr(colon + 1).c_str(), nullptr, 16);
			if (len_v == 0 || len_v > 0x10000)
			{
				std::fprintf(stderr, "[cellgov] CELLGOV_STORE_WATCH len out of range (1..0x10000): 0x%llx\n", len_v);
				return;
			}

			std::FILE* f = std::fopen(path, "wb");
			if (!f)
			{
				std::fprintf(stderr, "[cellgov] CELLGOV_STORE_WATCH_PATH open failed: %s\n", path);
				return;
			}

			const char magic[4] = {'C','G','S','W'};
			const u32 version = 1;
			const u32 watch_addr_u32 = static_cast<u32>(addr_v);
			const u32 watch_len_u32  = static_cast<u32>(len_v);
			std::fwrite(magic, 1, 4, f);
			std::fwrite(&version, 4, 1, f);
			std::fwrite(&watch_addr_u32, 4, 1, f);
			std::fwrite(&watch_len_u32, 4, 1, f);
			std::fflush(f);

			g_store_watch.watch_addr = watch_addr_u32;
			g_store_watch.watch_len  = watch_len_u32;
			g_store_watch.file       = f;
			g_store_watch.active.store(true, std::memory_order_release);

			std::fprintf(stderr, "[cellgov] store-watch active: addr=0x%x len=0x%x path=%s\n",
				watch_addr_u32, watch_len_u32, path);
		});
	}

	// Fast-path range check. `pc` is the storing instruction's PC;
	// `ea` is the absolute guest EA; `width` is the store width in
	// bytes (1, 2, 4, or 8); `value` is the stored value, zero-padded
	// to u64.
	inline void cellgov_store_watch_emit(u32 pc, u32 ea, u32 width, u64 value)
	{
		cellgov_store_watch_init_once();
		auto& s = g_store_watch;
		if (!s.active.load(std::memory_order_acquire)) return;
		// Range check: [ea, ea+width) overlaps [watch_addr, watch_addr+watch_len)
		if (ea + width <= s.watch_addr) return;
		if (ea >= s.watch_addr + s.watch_len) return;
		const u64 step = s.step.fetch_add(1, std::memory_order_relaxed);
		// 28 bytes: step u64, pc u32, ea u32, width u32, value u64
		std::lock_guard<std::mutex> lock(s.write_mu);
		if (!s.file) return;
		std::fwrite(&step, 8, 1, s.file);
		std::fwrite(&pc, 4, 1, s.file);
		std::fwrite(&ea, 4, 1, s.file);
		std::fwrite(&width, 4, 1, s.file);
		std::fwrite(&value, 8, 1, s.file);
	}

	// Sentinel pc values for non-PPU-store record kinds.
	//   0xFFFFFFFE = C++ vm::_ptr<T> deref (operator* / -> / [])
	//   0xFFFFFFFD = raw vm::base() pointer manipulation (not yet hooked)
	constexpr u32 PC_VM_PTR_DEREF = 0xFFFFFFFE;

	// vm::_ptr<T> dereference hook. Fires from operator*, operator->,
	// operator[] in vm_ptr.h when the resulting EA overlaps the watch
	// range. Records (step, PC_VM_PTR_DEREF, ea, sizeof(T), 0) -- value
	// unavailable at deref time (the assignment happens through the
	// returned reference AFTER this hook fires). Combine with
	// pre/post snapshots in analysis to determine which deref produced
	// the byte change.
	//
	// Coverage: this hook covers RPCS3 C++ code using vm::ptr<T> /
	// vm::_ptr<T> dereference syntax. Does NOT cover raw
	// `*reinterpret_cast<T*>(vm::base(addr))`, std::memcpy on
	// vm::base, or LLVM-recompiled guest stores (which are inlined
	// native and bypass all software hooks). An empty result is
	// "no vm::_ptr deref hit the watch range," NOT "no write."
	inline void cellgov_store_watch_emit_deref(u32 ea, u32 width)
	{
		cellgov_store_watch_init_once();
		auto& s = g_store_watch;
		if (!s.active.load(std::memory_order_acquire)) return;
		if (ea + width <= s.watch_addr) return;
		if (ea >= s.watch_addr + s.watch_len) return;
		const u64 step = s.step.fetch_add(1, std::memory_order_relaxed);
		std::lock_guard<std::mutex> lock(s.write_mu);
		if (!s.file) return;
		const u32 pc = PC_VM_PTR_DEREF;
		const u64 value = 0;
		std::fwrite(&step, 8, 1, s.file);
		std::fwrite(&pc, 4, 1, s.file);
		std::fwrite(&ea, 4, 1, s.file);
		std::fwrite(&width, 4, 1, s.file);
		std::fwrite(&value, 8, 1, s.file);
	}
}
