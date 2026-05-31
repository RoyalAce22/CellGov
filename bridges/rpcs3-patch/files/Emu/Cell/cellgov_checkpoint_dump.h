// CellGov checkpoint-dump hook (header-only). Env-var-gated dump of
// configured guest-memory regions to a host file. Two call sites use
// this body: the existing _sys_process_exit trigger and the new
// RSX-thread FirstRsxWrite trigger.
//
// Environment variables:
//
//   <path_env>           Path to output dump file for this trigger.
//                        When unset the call is a no-op. The file is
//                        overwritten on each run.
//
//   CELLGOV_DUMP_REGIONS Comma-separated list of `addr:size` pairs
//                        in hex (e.g.
//                        0x10000:0x800000,0x10000000:0x400000). Each
//                        region is appended to the dump file
//                        contiguously in declaration order. Shared
//                        across all triggers.
//
// Per-trigger path env vars:
//
//   CELLGOV_DUMP_PATH     ProcessExit dump (fires inside
//                         _sys_process_exit).
//   CELLGOV_DUMP_PATH_RSX FirstRsxWrite dump (fires inside the RSX
//                         thread's cpu_task loop when the guest's
//                         put register first differs from its
//                         initial value).
//   CELLGOV_DUMP_PATH_RSX2
//                         Second FirstRsxWrite dump (fires on the
//                         very next cpu_task iteration). Diff
//                         against the RSX1 dump is the tearing
//                         noise floor; see the README.
//
// Skews documented at the call sites:
//
// 1. Poll-observation skew. The guest store to the put field happens
//    on the PPU thread; the RSX thread notices via its poll loop,
//    possibly a few microseconds later.
//
// 2. Torn-read skew. The PPU is NOT halted while the RSX-thread dump
//    runs. The dump walks each region page-by-page; the PPU may
//    mutate other pages concurrently. Within one dump this is a torn
//    snapshot. Empirically zero for the standard manifest at
//    FirstRsxWrite (RSX1-vs-RSX2 diff was 0 bytes across 9.5 MiB on
//    WipEout); the actively-churning region at that moment is the
//    FIFO command buffer, not the data segment. The RSX1-vs-RSX2
//    diff is the empirical bound for any new title.
//
// 3. FIFO-drain skew. The trigger fires after the RSX thread has
//    already executed the first FIFO batch (it observes put != initial,
//    typically with put == get). CG traps on the guest store itself,
//    before any NV40 method runs. Nil for main-memory dumps unless the
//    first batch carries a method that writes guest memory
//    (SET_SEMAPHORE, NOTIFY, NV406E_SET_REFERENCE).
//
// Read-only with respect to guest state: the hook calls vm::base()
// + fwrite to the host output file and returns. No guest memory is
// modified. No scheduler perturbation.

#pragma once

#include "util/types.hpp"
#include "Emu/Memory/vm.h"

#include <atomic>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>

namespace cellgov
{
	// Walk `regions_env`'s comma-separated `addr:size` pairs, page-aware
	// (4 KiB), and append committed pages to `f`; write zeros for
	// unmapped pages so file offsets stay aligned with the requested
	// region boundaries. Returns total bytes written.
	inline void cellgov_write_regions(std::FILE* f, const char* regions_env)
	{
		std::string s(regions_env);
		std::size_t cursor = 0;
		while (cursor < s.size())
		{
			std::size_t comma = s.find(',', cursor);
			std::string pair = s.substr(cursor, comma == std::string::npos ? std::string::npos : comma - cursor);
			cursor = comma == std::string::npos ? s.size() : comma + 1;

			std::size_t colon = pair.find(':');
			if (colon == std::string::npos) continue;

			unsigned long long addr = std::strtoull(pair.substr(0, colon).c_str(), nullptr, 16);
			unsigned long long size = std::strtoull(pair.substr(colon + 1).c_str(), nullptr, 16);
			if (size == 0) continue;

			const u32 page_sz = 4096;
			u32 cur = static_cast<u32>(addr);
			const u32 end = static_cast<u32>(addr + size);
			u32 wrote = 0;
			u32 zeroed = 0;
			static const u8 zeros[page_sz] = {0};
			while (cur < end)
			{
				u32 chunk = page_sz - (cur % page_sz);
				if (chunk > end - cur) chunk = end - cur;
				if (vm::check_addr(cur, vm::page_readable, chunk))
				{
					std::fwrite(vm::base(cur), 1, chunk, f);
					wrote++;
				}
				else
				{
					std::fwrite(zeros, 1, chunk, f);
					zeroed++;
				}
				cur += chunk;
			}
			std::fprintf(stderr, "[cellgov] region 0x%llx:0x%llx wrote %u chunks, %u zero-filled\n",
				addr, size, wrote, zeroed);
		}
	}

	// Dump configured regions to the file named by `path_env`. No-op
	// when the env var is unset or empty, and when CELLGOV_DUMP_REGIONS
	// is unset or empty (with a stderr note in the latter case). Safe
	// to call concurrently with guest execution; the caller is
	// responsible for one-shot semantics (atomic_flag guard) since the
	// hook does not gate itself.
	inline void cellgov_checkpoint_dump(const char* path_env)
	{
		const char* path = std::getenv(path_env);
		if (!path || !*path)
		{
			return;
		}

		const char* regions_env = std::getenv("CELLGOV_DUMP_REGIONS");
		if (!regions_env || !*regions_env)
		{
			std::fprintf(stderr, "[cellgov] %s set but CELLGOV_DUMP_REGIONS empty; skipping dump\n", path_env);
			return;
		}

		std::FILE* f = std::fopen(path, "wb");
		if (!f)
		{
			std::fprintf(stderr, "[cellgov] failed to open dump path: %s\n", path);
			return;
		}

		cellgov_write_regions(f, regions_env);

		std::fclose(f);
		std::fprintf(stderr, "[cellgov] checkpoint dump (%s) written to %s\n", path_env, path);
	}
}

// CellGov store-watch hook (patch 0003 surface) lives in
// `Emu/Memory/cellgov_store_watch.h` so vm.h can include it without
// a circular dependency through this checkpoint-dump header (which
// itself needs vm::base / vm::check_addr). The split is a build-order
// detail; the two surfaces are otherwise unrelated.
