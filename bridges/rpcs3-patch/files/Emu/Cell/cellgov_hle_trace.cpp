// See cellgov_hle_trace.h for the public contract and file format.

#include "stdafx.h"
#include "cellgov_hle_trace.h"

#include "Emu/Cell/PPUThread.h"
#include "Emu/Memory/vm.h"

#include <atomic>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <mutex>
#include <string>
#include <vector>

namespace cellgov_hle_trace
{
	namespace
	{
		struct WatchRegion
		{
			u64 addr;
			u32 size;
		};

		struct Frame
		{
			const char* name;
			u64 step;
			u64 lr;
			u64 args[8];
			u32 thread_id;
			// Snapshot of all watch regions at entry, concatenated
			// in declaration order.
			std::vector<u8> snapshot;
		};

		std::atomic<bool> g_initialized{false};
		std::mutex g_init_mutex;

		bool g_active = false;
		std::FILE* g_trace_file = nullptr;
		std::mutex g_emit_mutex;
		std::vector<WatchRegion> g_watch;
		// Bytes already emitted to the trace file. Compared against
		// `g_max_bytes` on every emit so a runaway trace cannot fill
		// the host disk -- discovered the hard way after a 2-day
		// untended run produced an 80 GB trace.
		std::atomic<unsigned long long> g_emitted_bytes{0};
		unsigned long long g_max_bytes = 0;

		// Per-thread call stack. Tracks nested HLE calls so each
		// level emits its own record.
		thread_local std::vector<Frame> tls_stack;

		// Per-thread snapshot of watch regions taken at the most
		// recent HLE-call EXIT. Used to detect guest-code writes
		// that happen BETWEEN HLE calls (the JIT compiles guest
		// stores inline, bypassing vm::write*, so the only signal
		// available to a host-side hook is the watch-region drift
		// across the inter-call interval). When the next enter()
		// observes a different value, the diff is attributed to a
		// synthetic "<guest_code>" record.
		thread_local std::vector<u8> tls_last_exit_snapshot;
		thread_local bool tls_have_last_exit = false;
		thread_local u64 tls_last_exit_step = 0;
		thread_local u64 tls_last_exit_lr = 0;
		thread_local u32 tls_last_exit_thread_id = 0;

		// Parse a comma-separated list of `addr:size` pairs in hex.
		// Tolerates 0x-prefix and bare hex. Silently drops malformed
		// entries -- this is investigation tooling, not a parser
		// suitable for production input.
		void parse_watch_list(const char* env, std::vector<WatchRegion>& out)
		{
			std::string s(env);
			size_t pos = 0;
			while (pos < s.size())
			{
				size_t comma = s.find(',', pos);
				if (comma == std::string::npos) comma = s.size();
				std::string entry = s.substr(pos, comma - pos);
				pos = comma + 1;

				size_t colon = entry.find(':');
				if (colon == std::string::npos) continue;
				std::string addr_s = entry.substr(0, colon);
				std::string size_s = entry.substr(colon + 1);

				auto strip_prefix = [](std::string& v) {
					if (v.size() >= 2 && v[0] == '0' && (v[1] == 'x' || v[1] == 'X'))
						v = v.substr(2);
				};
				strip_prefix(addr_s);
				strip_prefix(size_s);

				try
				{
					u64 addr = std::stoull(addr_s, nullptr, 16);
					u64 size = std::stoull(size_s, nullptr, 16);
					if (size == 0 || size > (1u << 20)) continue; // sanity cap 1 MiB
					out.push_back({addr, static_cast<u32>(size)});
				}
				catch (...)
				{
					// malformed entry; drop
				}
			}
		}

		void initialize_locked()
		{
			const char* path = std::getenv("CELLGOV_HLE_TRACE_PATH");
			if (!path || !*path)
			{
				g_active = false;
				return;
			}

			g_trace_file = std::fopen(path, "wb");
			if (!g_trace_file)
			{
				g_active = false;
				return;
			}

			// Header.
			const u32 header_magic = 0xC0E60001u;
			const u32 version = 2;
			std::fwrite(&header_magic, sizeof(u32), 1, g_trace_file);
			std::fwrite(&version, sizeof(u32), 1, g_trace_file);

			const char* watch = std::getenv("CELLGOV_HLE_WATCH");
			if (watch && *watch)
			{
				parse_watch_list(watch, g_watch);
			}

			// Self-cap to keep an unattended run from filling the
			// host disk. Default 2 GiB; override via env var. A run
			// that exits cleanly produces a trace well under that.
			const char* cap = std::getenv("CELLGOV_HLE_TRACE_MAX_MB");
			unsigned long long max_mb = (cap && *cap) ? std::strtoull(cap, nullptr, 10) : 2048ULL;
			if (max_mb == 0) max_mb = 2048ULL;
			g_max_bytes = max_mb * 1024ULL * 1024ULL;

			g_active = true;
		}

		// Read `size` bytes at guest address `addr` into `out`. Best
		// effort: returns false (and zeros `out`) if the access
		// would fault. We use vm::check_addr to gate; bytes copied
		// via vm::base + addr (aliased mapping).
		bool read_guest(u64 addr, u32 size, std::vector<u8>& out)
		{
			out.assign(size, 0);
			if (!vm::check_addr(static_cast<u32>(addr), vm::page_readable, size))
			{
				return false;
			}
			std::memcpy(out.data(), vm::base(static_cast<u32>(addr)), size);
			return true;
		}

		// Emit a synthetic record attributing watch-region drift
		// between `prev_step` (last HLE exit) and `cur_step` (this
		// HLE entry) to guest-code stores. `name = "<guest_code>"`
		// so the consumer can group separately from real HLE
		// records.
		void emit_guest_drift(u64 prev_step, u64 prev_lr, u64 cur_step, u32 thread_id,
			const std::vector<u8>& before, const std::vector<u8>& after)
		{
			// Build per-watch-region writes for the bytes that
			// actually changed.
			struct WriteOut { u64 addr; std::vector<u8> bytes; };
			std::vector<WriteOut> writes;
			size_t off = 0;
			for (const auto& w : g_watch)
			{
				bool changed = false;
				for (u32 i = 0; i < w.size; ++i)
				{
					if (after[off + i] != before[off + i])
					{
						changed = true;
						break;
					}
				}
				if (changed)
				{
					std::vector<u8> bytes(after.begin() + off,
						after.begin() + off + w.size);
					writes.push_back(WriteOut{w.addr, std::move(bytes)});
				}
				off += w.size;
			}
			if (writes.empty()) return;

			std::lock_guard<std::mutex> lock(g_emit_mutex);
			if (!g_trace_file) return;
			if (g_emitted_bytes.load(std::memory_order_relaxed) >= g_max_bytes)
			{
				static bool warned = false;
				if (!warned)
				{
					std::fprintf(stderr, "[cellgov] hle-trace cap (%llu MB) reached; dropping further records\n",
						(unsigned long long)(g_max_bytes / (1024ULL * 1024ULL)));
					warned = true;
				}
				return;
			}

			const u32 record_magic = 0xC0E60002u;
			const u32 depth = 0;
			const char* name = "<guest_code>";
			const u32 name_len = static_cast<u32>(std::strlen(name));
			const u64 args[8] = {prev_step, cur_step, 0, 0, 0, 0, 0, 0};
			const u64 ret = 0;

			std::fwrite(&record_magic, sizeof(u32), 1, g_trace_file);
			// Use cur_step (this HLE call's entry CIA) as the
			// step. lr field carries prev_lr so an investigator
			// can locate the user-code site that ran in the gap.
			std::fwrite(&cur_step, sizeof(u64), 1, g_trace_file);
			std::fwrite(&prev_lr, sizeof(u64), 1, g_trace_file);
			std::fwrite(&thread_id, sizeof(u32), 1, g_trace_file);
			std::fwrite(&depth, sizeof(u32), 1, g_trace_file);
			std::fwrite(&name_len, sizeof(u32), 1, g_trace_file);
			std::fwrite(name, 1, name_len, g_trace_file);
			std::fwrite(args, sizeof(u64), 8, g_trace_file);
			std::fwrite(&ret, sizeof(u64), 1, g_trace_file);
			const u32 num_writes = static_cast<u32>(writes.size());
			std::fwrite(&num_writes, sizeof(u32), 1, g_trace_file);
			for (const auto& w : writes)
			{
				const u32 size = static_cast<u32>(w.bytes.size());
				std::fwrite(&w.addr, sizeof(u64), 1, g_trace_file);
				std::fwrite(&size, sizeof(u32), 1, g_trace_file);
				std::fwrite(w.bytes.data(), 1, size, g_trace_file);
			}
			std::fflush(g_trace_file);
			// Best-effort byte tally: a record is at minimum
			// 4+8+8+4+4+4+name_len+64+8+4 + per-write 8+4+size.
			// We approximate by ftell() if available; here just
			// count the conservatively-known fixed portion +
			// payload.
			unsigned long long fixed = 4 + 8 + 8 + 4 + 4 + 4 + name_len + 64 + 8 + 4;
			unsigned long long payload = 0;
			for (const auto& w : writes) payload += 8ULL + 4ULL + w.bytes.size();
			g_emitted_bytes.fetch_add(fixed + payload, std::memory_order_relaxed);
		}
	} // anonymous namespace

	void ensure_initialized()
	{
		if (g_initialized.load(std::memory_order_acquire)) return;
		std::lock_guard<std::mutex> lock(g_init_mutex);
		if (g_initialized.load(std::memory_order_relaxed)) return;
		initialize_locked();
		g_initialized.store(true, std::memory_order_release);
	}

	bool is_active()
	{
		// Callers wrap with ensure_initialized first; reading the
		// flag without ordering is fine because writers publish
		// once via initialize_locked().
		return g_active;
	}

	void enter(ppu_thread& ppu, const char* name, const u64* args)
	{
		Frame f{};
		f.name = name;
		f.step = ppu.cia;
		f.lr = ppu.lr;
		f.thread_id = ppu.id;
		std::memcpy(f.args, args, sizeof(f.args));

		if (!g_watch.empty())
		{
			// Concatenate every watch region's bytes into one
			// snapshot buffer. Layout matches the diff phase below.
			size_t total = 0;
			for (const auto& w : g_watch) total += w.size;
			f.snapshot.resize(total);
			size_t off = 0;
			std::vector<u8> tmp;
			for (const auto& w : g_watch)
			{
				read_guest(w.addr, w.size, tmp);
				std::memcpy(f.snapshot.data() + off, tmp.data(), w.size);
				off += w.size;
			}

			// Compare the entry snapshot against the previous
			// HLE-call's exit snapshot for this thread. Any drift
			// is from guest-code stores that ran between the two
			// calls; emit a synthetic "<guest_code>" record so the
			// consumer can attribute those writes alongside HLE
			// ones. Only fires at depth 0 (outermost call) -- nested
			// calls cannot have guest-code intervals between them.
			if (tls_have_last_exit && tls_stack.empty()
				&& tls_last_exit_thread_id == f.thread_id
				&& tls_last_exit_snapshot.size() == f.snapshot.size())
			{
				bool changed = false;
				for (size_t i = 0; i < f.snapshot.size(); ++i)
				{
					if (f.snapshot[i] != tls_last_exit_snapshot[i])
					{
						changed = true;
						break;
					}
				}
				if (changed)
				{
					emit_guest_drift(
						tls_last_exit_step,
						tls_last_exit_lr,
						f.step,
						f.thread_id,
						tls_last_exit_snapshot,
						f.snapshot
					);
				}
			}
		}

		tls_stack.push_back(std::move(f));
	}

	void exit(ppu_thread& ppu, u64 ret)
	{
		if (tls_stack.empty()) return; // defensive; should not happen
		Frame f = std::move(tls_stack.back());
		const u32 depth = static_cast<u32>(tls_stack.size() - 1);
		tls_stack.pop_back();

		// Compute writes by diffing snapshot vs current bytes.
		struct WriteOut
		{
			u64 addr;
			std::vector<u8> bytes;
		};
		std::vector<WriteOut> writes;
		if (!g_watch.empty())
		{
			size_t off = 0;
			std::vector<u8> tmp;
			for (const auto& w : g_watch)
			{
				read_guest(w.addr, w.size, tmp);
				bool changed = false;
				for (u32 i = 0; i < w.size; ++i)
				{
					if (tmp[i] != f.snapshot[off + i])
					{
						changed = true;
						break;
					}
				}
				if (changed)
				{
					writes.push_back(WriteOut{w.addr, std::move(tmp)});
				}
				off += w.size;
			}
		}

		// Emit one record. Single-writer mutex around fwrite so
		// nested-thread interleaving is safe.
		std::lock_guard<std::mutex> lock(g_emit_mutex);
		if (!g_trace_file) return;
		if (g_emitted_bytes.load(std::memory_order_relaxed) >= g_max_bytes)
		{
			static bool warned = false;
			if (!warned)
			{
				std::fprintf(stderr, "[cellgov] hle-trace cap (%llu MB) reached; dropping further records\n",
					(unsigned long long)(g_max_bytes / (1024ULL * 1024ULL)));
				warned = true;
			}
			return;
		}

		const u32 record_magic = 0xC0E60002u;
		std::fwrite(&record_magic, sizeof(u32), 1, g_trace_file);
		std::fwrite(&f.step, sizeof(u64), 1, g_trace_file);
		std::fwrite(&f.lr, sizeof(u64), 1, g_trace_file);
		std::fwrite(&f.thread_id, sizeof(u32), 1, g_trace_file);
		std::fwrite(&depth, sizeof(u32), 1, g_trace_file);

		const u32 name_len = static_cast<u32>(std::strlen(f.name));
		std::fwrite(&name_len, sizeof(u32), 1, g_trace_file);
		std::fwrite(f.name, 1, name_len, g_trace_file);
		std::fwrite(f.args, sizeof(u64), 8, g_trace_file);
		std::fwrite(&ret, sizeof(u64), 1, g_trace_file);

		const u32 num_writes = static_cast<u32>(writes.size());
		std::fwrite(&num_writes, sizeof(u32), 1, g_trace_file);
		for (const auto& w : writes)
		{
			const u32 size = static_cast<u32>(w.bytes.size());
			std::fwrite(&w.addr, sizeof(u64), 1, g_trace_file);
			std::fwrite(&size, sizeof(u32), 1, g_trace_file);
			std::fwrite(w.bytes.data(), 1, size, g_trace_file);
		}

		// Flush per record so a crash mid-boot still preserves the
		// records emitted so far. Investigation tool, not a hot
		// path -- correctness wins over throughput.
		std::fflush(g_trace_file);
		{
			unsigned long long fixed = 4 + 8 + 8 + 4 + 4 + 4 + name_len + 64 + 8 + 4;
			unsigned long long payload = 0;
			for (const auto& w : writes) payload += 8ULL + 4ULL + w.bytes.size();
			g_emitted_bytes.fetch_add(fixed + payload, std::memory_order_relaxed);
		}

		// Save the EXIT snapshot at depth 0 so the next outer-call
		// entry can detect guest-code drift across the gap. Take a
		// fresh read here (not the per-write diff vector above) so
		// the snapshot is the full watch region in canonical order.
		if (!g_watch.empty() && tls_stack.empty())
		{
			size_t total = 0;
			for (const auto& w : g_watch) total += w.size;
			tls_last_exit_snapshot.resize(total);
			size_t off = 0;
			std::vector<u8> tmp;
			for (const auto& w : g_watch)
			{
				read_guest(w.addr, w.size, tmp);
				std::memcpy(tls_last_exit_snapshot.data() + off, tmp.data(), w.size);
				off += w.size;
			}
			tls_have_last_exit = true;
			tls_last_exit_step = f.step;
			tls_last_exit_lr = ppu.lr;
			tls_last_exit_thread_id = f.thread_id;
		}
	}
}
