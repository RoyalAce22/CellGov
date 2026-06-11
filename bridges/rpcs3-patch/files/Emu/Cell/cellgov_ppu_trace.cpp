// See cellgov_ppu_trace.h for the public contract and file format.

#include "stdafx.h"
#include "cellgov_ppu_trace.h"

#include "Emu/Cell/PPUThread.h"
#include "Emu/Memory/vm.h"

#include <atomic>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <mutex>
#include <string>
#include <vector>

namespace cellgov_ppu_trace
{
	namespace
	{
		// Inclusive PC range from CELLGOV_PPU_TRACE_PC_RANGE.
		struct PcRange
		{
			u64 lo = 0;
			u64 hi = 0;
			bool active = false;
		};

		std::atomic<bool> g_initialized{false};
		std::mutex g_init_mutex;

		bool g_active = false;
		std::FILE* g_file = nullptr;
		std::mutex g_emit_mutex;

		PcRange g_pc_range;
		std::vector<u8> g_primary_filter; // primary opcode whitelist; empty = no filter
		bool g_has_primary_filter = false;

		std::atomic<unsigned long long> g_emitted_records{0};
		std::atomic<unsigned long long> g_emitted_bytes{0};
		unsigned long long g_max_records = 0; // 0 == unlimited
		unsigned long long g_max_bytes = 0;   // 0 == unlimited

		// Pre-state captured at on_pre_dispatch, consumed at emit_record.
		// Per-thread so concurrent PPU threads can capture independently.
		struct PreState
		{
			u64 pc = 0;
			u32 raw = 0;
			u32 thread_id = 0;
			u64 gpr[32]{};
			u64 fpr[32]{};
			u8 vr[32][16]{};
			u32 cr = 0;
			u64 lr = 0;
			u64 ctr = 0;
			u64 xer = 0;
			u32 raddr = 0;          // RPCS3 ppu_thread::raddr at entry
			u64 rtime = 0;          // RPCS3 ppu_thread::rtime at entry
			u64 mem_addr = 0;       // populated by classify_mem_access
			u32 mem_len = 0;        // populated by classify_mem_access
			std::vector<u8> mem_pre; // populated when mem_len > 0
			bool captured = false;
		};

		thread_local PreState tls_pre;

		// Memory-access shape for one instruction. `len == 0` means the
		// instruction did not touch guest memory (or the classifier did
		// not recognize it).
		struct MemAccess
		{
			u64 addr;
			u32 len;
		};

		// Compute the memory window an instruction will touch from the
		// PPU pre-state. Returns `len = 0` for non-memory instructions
		// and for memory ops whose form is not yet classified (the
		// record then carries no mem_pre / mem_post). Covers the
		// common load / store families: D-form (primaries 32..55), DS-
		// form (58 / 62), X-form scalar + FP + AltiVec-memory + Cell-
		// unaligned VXU + byte-reverse + atomic + dcbz under primary
		// 31. The classifier mirrors CellGov's decode tables; if an
		// encoding is missing here a future capture will land with
		// len=0 rather than mis-sized bytes.
		MemAccess classify_mem_access(const ppu_thread& ppu, u32 raw)
		{
			const u8 primary = static_cast<u8>((raw >> 26) & 0x3f);
			const u8 ra = static_cast<u8>((raw >> 16) & 0x1f);
			const u8 rb = static_cast<u8>((raw >> 11) & 0x1f);
			const s16 d_imm = static_cast<s16>(raw & 0xffff);
			const u64 ra_val = (ra == 0) ? 0 : ppu.gpr[ra];

			auto d_form = [&](u32 size) -> MemAccess {
				const s64 ea = static_cast<s64>(ra_val) + static_cast<s64>(d_imm);
				return MemAccess{ static_cast<u64>(ea), size };
			};

			switch (primary)
			{
			case 32: case 33: return d_form(4); // lwz/lwzu
			case 34: case 35: return d_form(1); // lbz/lbzu
			case 36: case 37: return d_form(4); // stw/stwu
			case 38: case 39: return d_form(1); // stb/stbu
			case 40: case 41: return d_form(2); // lhz/lhzu
			case 42: case 43: return d_form(2); // lha/lhau
			case 44: case 45: return d_form(2); // sth/sthu
			case 48: case 49: return d_form(4); // lfs/lfsu
			case 50: case 51: return d_form(8); // lfd/lfdu
			case 52: case 53: return d_form(4); // stfs/stfsu
			case 54: case 55: return d_form(8); // stfd/stfdu
			case 46: case 47:
			{
				// lmw / stmw store words for r=RT/RS .. 31. Window is
				// (32 - RT/RS) words = (32 - RT/RS) * 4 bytes.
				const u8 rt_or_rs = static_cast<u8>((raw >> 21) & 0x1f);
				const s64 ea = static_cast<s64>(ra_val) + static_cast<s64>(d_imm);
				return MemAccess{ static_cast<u64>(ea), static_cast<u32>(32u - rt_or_rs) * 4u };
			}
			case 58:
			{
				// DS-form: low 2 bits select ld / ldu / lwa. Imm is the
				// 14-bit DS shifted left 2.
				const s16 ds_imm = static_cast<s16>(raw & 0xfffc);
				const u8 sub = static_cast<u8>(raw & 0x3);
				const u32 size = (sub == 2) ? 4u : 8u; // lwa is 4, ld / ldu are 8
				const s64 ea = static_cast<s64>(ra_val) + static_cast<s64>(ds_imm);
				return MemAccess{ static_cast<u64>(ea), size };
			}
			case 62:
			{
				const s16 ds_imm = static_cast<s16>(raw & 0xfffc);
				const s64 ea = static_cast<s64>(ra_val) + static_cast<s64>(ds_imm);
				return MemAccess{ static_cast<u64>(ea), 8u };
			}
			case 31:
			{
				const u16 xo = static_cast<u16>((raw >> 1) & 0x3ff);
				const u64 rb_val = ppu.gpr[rb];
				const u64 ea = ra_val + rb_val;
				auto x = [&](u64 mask, u32 size) -> MemAccess {
					return MemAccess{ ea & mask, size };
				};
				switch (xo)
				{
				case 23: case 55: return x(~0ull, 4);   // lwzx/lwzux
				case 87: case 119: return x(~0ull, 1);  // lbzx/lbzux
				case 279: case 311: return x(~0ull, 2); // lhzx/lhzux
				case 21: case 53: return x(~0ull, 8);   // ldx/ldux
				case 343: case 375: return x(~0ull, 2); // lhax/lhaux
				case 341: case 373: return x(~0ull, 4); // lwax/lwaux
				case 151: case 183: return x(~0ull, 4); // stwx/stwux
				case 215: case 247: return x(~0ull, 1); // stbx/stbux
				case 407: case 439: return x(~0ull, 2); // sthx/sthux
				case 149: case 181: return x(~0ull, 8); // stdx/stdux
				case 532: return x(~0ull, 8);           // ldbrx
				case 534: return x(~0ull, 4);           // lwbrx
				case 790: return x(~0ull, 2);           // lhbrx
				case 660: return x(~0ull, 8);           // sdbrx
				case 662: return x(~0ull, 4);           // stwbrx
				case 918: return x(~0ull, 2);           // sthbrx
				case 7: return x(~0ull, 1);             // lvebx
				case 39: return x(~1ull, 2);            // lvehx
				case 71: return x(~3ull, 4);            // lvewx
				case 135: return x(~0ull, 1);           // stvebx
				case 167: return x(~1ull, 2);           // stvehx
				case 199: return x(~3ull, 4);           // stvewx
				case 103: case 359: return x(~15ull, 16); // lvx/lvxl
				case 231: case 487: return x(~15ull, 16); // stvx/stvxl
				case 519: case 583: case 647: case 711: return x(~15ull, 16); // lvlx/lvrx/lvlxl/lvrxl
				case 775: case 839: case 903: case 967: return x(~15ull, 16); // stvlx/stvrx/stvlxl/stvrxl
				case 535: case 567: return x(~0ull, 4); // lfsx/lfsux
				case 599: case 631: return x(~0ull, 8); // lfdx/lfdux
				case 663: case 695: return x(~0ull, 4); // stfsx/stfsux
				case 727: case 759: return x(~0ull, 8); // stfdx/stfdux
				case 983: return x(~0ull, 4);           // stfiwx
				case 1014: return x(~127ull, 128);      // dcbz
				case 20: return x(~0ull, 4);            // lwarx
				case 84: return x(~0ull, 8);            // ldarx
				case 150: return x(~0ull, 4);           // stwcx.
				case 214: return x(~0ull, 8);           // stdcx.
				default: return MemAccess{ 0, 0 };      // lvsl/lvsr/permute/etc.
				}
			}
			default: return MemAccess{ 0, 0 };
			}
		}

		void write_u32(u32 v)
		{
			u8 buf[4] = {
				static_cast<u8>(v & 0xff),
				static_cast<u8>((v >> 8) & 0xff),
				static_cast<u8>((v >> 16) & 0xff),
				static_cast<u8>((v >> 24) & 0xff),
			};
			std::fwrite(buf, 1, 4, g_file);
			g_emitted_bytes.fetch_add(4, std::memory_order_relaxed);
		}

		void write_u64(u64 v)
		{
			u8 buf[8];
			for (int i = 0; i < 8; ++i)
				buf[i] = static_cast<u8>((v >> (8 * i)) & 0xff);
			std::fwrite(buf, 1, 8, g_file);
			g_emitted_bytes.fetch_add(8, std::memory_order_relaxed);
		}

		void write_bytes(const u8* p, std::size_t n)
		{
			std::fwrite(p, 1, n, g_file);
			g_emitted_bytes.fetch_add(n, std::memory_order_relaxed);
		}

		// Parse a single "start-end" hex range, tolerating 0x prefixes
		// and trailing junk. Returns false on parse failure.
		bool parse_pc_range(const char* env, PcRange& out)
		{
			std::string s(env);
			std::size_t dash = s.find('-');
			if (dash == std::string::npos) return false;
			out.lo = std::strtoull(s.substr(0, dash).c_str(), nullptr, 16);
			out.hi = std::strtoull(s.substr(dash + 1).c_str(), nullptr, 16);
			out.active = true;
			return true;
		}

		// Parse a comma-separated list of decimal primary opcodes.
		void parse_primary_filter(const char* env, std::vector<u8>& out)
		{
			std::string s(env);
			std::size_t cursor = 0;
			while (cursor < s.size())
			{
				std::size_t comma = s.find(',', cursor);
				std::string tok = s.substr(cursor, comma == std::string::npos ? std::string::npos : comma - cursor);
				cursor = comma == std::string::npos ? s.size() : comma + 1;
				if (tok.empty()) continue;
				int v = std::atoi(tok.c_str());
				if (v >= 0 && v <= 63) out.push_back(static_cast<u8>(v));
			}
		}

		bool primary_passes_filter(u32 raw)
		{
			if (!g_has_primary_filter) return true;
			u8 primary = static_cast<u8>((raw >> 26) & 0x3f);
			for (u8 p : g_primary_filter)
				if (p == primary) return true;
			return false;
		}

		// Pack the ppu_thread's XER bits into the u64 representation
		// CellGov's PpuState::xer uses (bit 31 = SO, bit 30 = OV,
		// bit 29 = CA, bits 0..6 = CNT). The struct in PPUThread.h
		// stores each XER field as a separate scalar, so the packing
		// is done explicitly here rather than via a member method.
		u64 pack_xer(const ppu_thread& ppu)
		{
			u64 r = 0;
			if (ppu.xer.so) r |= (1ull << 31);
			if (ppu.xer.ov) r |= (1ull << 30);
			if (ppu.xer.ca) r |= (1ull << 29);
			r |= static_cast<u64>(ppu.xer.cnt & 0x7f);
			return r;
		}

		bool pc_passes_filter(u64 pc)
		{
			if (!g_pc_range.active) return true;
			return pc >= g_pc_range.lo && pc <= g_pc_range.hi;
		}

		bool budget_exhausted()
		{
			if (g_max_records && g_emitted_records.load(std::memory_order_relaxed) >= g_max_records)
				return true;
			if (g_max_bytes && g_emitted_bytes.load(std::memory_order_relaxed) >= g_max_bytes)
				return true;
			return false;
		}
	}

	void ensure_initialized()
	{
		bool expected = false;
		if (!g_initialized.compare_exchange_strong(expected, true))
			return;

		std::lock_guard<std::mutex> lk(g_init_mutex);

		const char* path = std::getenv("CELLGOV_PPU_TRACE_PATH");
		if (!path || !*path) return;

		g_file = std::fopen(path, "wb");
		if (!g_file)
		{
			std::fprintf(stderr, "[cellgov_ppu_trace] failed to open %s\n", path);
			return;
		}

		if (const char* env = std::getenv("CELLGOV_PPU_TRACE_PC_RANGE"))
			parse_pc_range(env, g_pc_range);

		if (const char* env = std::getenv("CELLGOV_PPU_TRACE_PRIMARY"))
		{
			parse_primary_filter(env, g_primary_filter);
			g_has_primary_filter = !g_primary_filter.empty();
		}

		if (const char* env = std::getenv("CELLGOV_PPU_TRACE_MAX_RECORDS"))
			g_max_records = std::strtoull(env, nullptr, 10);

		if (const char* env = std::getenv("CELLGOV_PPU_TRACE_MAX_BYTES"))
			g_max_bytes = std::strtoull(env, nullptr, 10);

		// Emit the file header.
		write_u32(HEADER_MAGIC);
		write_u32(FORMAT_VERSION);

		g_active = true;
		std::fprintf(stderr,
			"[cellgov_ppu_trace] active: path=%s pc_range=%s primary_filter=%zu max_records=%llu max_bytes=%llu\n",
			path,
			g_pc_range.active ? "yes" : "no",
			g_primary_filter.size(),
			static_cast<unsigned long long>(g_max_records),
			static_cast<unsigned long long>(g_max_bytes));
	}

	bool is_active()
	{
		return g_active;
	}

	void on_pre_dispatch(ppu_thread& ppu, u64 pc, u32 raw)
	{
		if (!g_active) return;
		if (budget_exhausted()) return;
		if (!pc_passes_filter(pc)) { tls_pre.captured = false; return; }
		if (!primary_passes_filter(raw)) { tls_pre.captured = false; return; }

		tls_pre.pc = pc;
		tls_pre.raw = raw;
		tls_pre.thread_id = static_cast<u32>(ppu.id);

		for (int i = 0; i < 32; ++i)
		{
			tls_pre.gpr[i] = ppu.gpr[i];
			tls_pre.fpr[i] = std::bit_cast<u64>(ppu.fpr[i]);
			// Vector lane storage: RPCS3 stores VRs as v128 (16 bytes).
			// Copy bytes as-is (host endianness matches the spec's
			// byte-0-MSB convention because PPU is big-endian).
			std::memcpy(&tls_pre.vr[i][0], &ppu.vr[i], 16);
		}
		tls_pre.cr = ppu.cr.pack();
		tls_pre.lr = ppu.lr;
		tls_pre.ctr = ppu.ctr;
		tls_pre.xer = pack_xer(ppu);
		tls_pre.raddr = ppu.raddr;
		tls_pre.rtime = ppu.rtime;

		const MemAccess access = classify_mem_access(ppu, raw);
		tls_pre.mem_addr = access.addr;
		tls_pre.mem_len = access.len;
		tls_pre.mem_pre.clear();
		if (access.len > 0)
		{
			tls_pre.mem_pre.resize(access.len);
			for (u32 i = 0; i < access.len; ++i)
			{
				tls_pre.mem_pre[i] = vm::read8(static_cast<u32>(access.addr + i));
			}
		}

		tls_pre.captured = true;
	}

	void emit_record(ppu_thread& ppu)
	{
		if (!tls_pre.captured) return;
		if (budget_exhausted()) return;

		std::lock_guard<std::mutex> lk(g_emit_mutex);
		if (budget_exhausted()) return;

		write_u32(RECORD_MAGIC);
		write_u64(tls_pre.pc);
		write_u32(tls_pre.raw);
		write_u32(tls_pre.thread_id);

		for (int i = 0; i < 32; ++i) write_u64(tls_pre.gpr[i]);
		for (int i = 0; i < 32; ++i) write_u64(tls_pre.fpr[i]);
		for (int i = 0; i < 32; ++i) write_bytes(tls_pre.vr[i], 16);
		write_u32(tls_pre.cr);
		write_u64(tls_pre.lr);
		write_u64(tls_pre.ctr);
		write_u64(tls_pre.xer);
		write_u32(tls_pre.raddr);
		write_u64(tls_pre.rtime);

		for (int i = 0; i < 32; ++i) write_u64(ppu.gpr[i]);
		for (int i = 0; i < 32; ++i) write_u64(std::bit_cast<u64>(ppu.fpr[i]));
		for (int i = 0; i < 32; ++i)
		{
			u8 buf[16];
			std::memcpy(buf, &ppu.vr[i], 16);
			write_bytes(buf, 16);
		}
		write_u32(ppu.cr.pack());
		write_u64(ppu.lr);
		write_u64(ppu.ctr);
		write_u64(pack_xer(ppu));
		write_u32(ppu.raddr);
		write_u64(ppu.rtime);

		write_u64(tls_pre.mem_addr);
		write_u32(tls_pre.mem_len);
		if (tls_pre.mem_len > 0)
		{
			write_bytes(tls_pre.mem_pre.data(), tls_pre.mem_len);
			std::vector<u8> post(tls_pre.mem_len);
			for (u32 i = 0; i < tls_pre.mem_len; ++i)
			{
				post[i] = vm::read8(static_cast<u32>(tls_pre.mem_addr + i));
			}
			write_bytes(post.data(), tls_pre.mem_len);
		}

		tls_pre.captured = false;
		tls_pre.mem_pre.clear();
		g_emitted_records.fetch_add(1, std::memory_order_relaxed);
	}
}
