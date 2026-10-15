/* SPU program for spu_double_policy: how double-precision arithmetic
 * and the two precision conversions treat denormal operands, denormal
 * results, NaN operands and NaN results.
 *
 * Every case writes the FPSCR with only the two double-precision
 * rounding fields set (both slices in one mode), runs one instruction
 * with the same operands in both doubleword slots, and records 16
 * words:
 *
 *   +0  op (the enum below)     +4  rounding mode (0 RN, 1 RZ, 2 +inf, 3 -inf)
 *   +8  A (2 words)             +16 B (2 words)     +24 C (2 words)
 *   +32 the result quadword (4 words)
 *   +48 the FPSCR after the instruction (4 words)
 *
 * For fesd, A's high word is the single in word slots 0 and 2. For
 * frds, A is the double and the result is the single in word slots 0
 * and 2. For dfma, dfms, dfnms and dfnma, C is the RT operand.
 *
 *   1. Denormal operands, RN: for D in 0x000FFFFFFFFFFFFF,
 *      0x0000000000000001 and 0x800FFFFFFFFFFFFF, 13 cases each with D
 *      in every operand position of dfa, dfs, dfm and the fused forms.
 *      39 cases.
 *   2. Denormal results, each in the four modes: dfm(2^-1000, 2^-40),
 *      dfm(2^-1000 (1 + 2^-52), 1.5 x 2^-40), dfm(2^-1000, 2^-80) and
 *      dfs(1.5 x 2^-1022, 2^-1022). 16 cases.
 *   3. NaNs, RN: 21 cases of dfa, dfm and dfma with QNaN and SNaN
 *      operands in each position, with payloads and both signs, and
 *      two invalid operations with no NaN input.
 *   4. Conversions: fesd of 0xFF800001, 0x7FC12345, 0x00000001 and
 *      0x807FFFFF (RN); frds of 0xFFF0000000000001, 0x7FF8123456789ABC
 *      and 0x000FFFFFFFFFFFFF (RN); frds of 1.2 x 2^-140 and of 2^1000
 *      in the four modes. 15 cases.
 *
 * Result layout: the 91 cases of 64 bytes, then zero padding to
 * 6144 bytes, DMA'd to the EA in argp.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define DMA_TAG 0
#define OUT_BYTES 6144

typedef unsigned long long u64;

static unsigned int out[OUT_BYTES / 4] __attribute__((aligned(128)));
static unsigned int n_cases;

enum op { DFA, DFS, DFM, DFMA, DFMS, DFNMS, DFNMA, FESD, FRDS };

/* Both doubleword slots hold x; the asm hides it from the compiler, so
 * nothing is folded at compile time under the host's IEEE rules. */
static inline qword d2(u64 x)
{
    vec_ullong2 v = spu_splats(x);
    __asm__ ("" : "+r"(v));
    return (qword)v;
}

/* Word slots 0 and 2 hold x. */
static inline qword s02(unsigned int x)
{
    vec_uint4 v = (vec_uint4){ x, 0, x, 0 };
    __asm__ ("" : "+r"(v));
    return (qword)v;
}

static void __attribute__((noinline)) record(enum op op, unsigned int rm, u64 a, u64 b, u64 c)
{
    unsigned int *d = out + n_cases++ * 16;
    qword r, fpscr;
    qword ra = op == FESD ? s02((unsigned int)(a >> 32)) : d2(a);
    qword rb = d2(b);
    qword rt = d2(c);

    spu_mtfpscr(((vec_uint4){ rm << 10 | rm << 8, 0, 0, 0 }));
    switch (op) {
    case DFA:   __asm__ volatile ("dfa %0,%1,%2" : "=r"(r) : "r"(ra), "r"(rb)); break;
    case DFS:   __asm__ volatile ("dfs %0,%1,%2" : "=r"(r) : "r"(ra), "r"(rb)); break;
    case DFM:   __asm__ volatile ("dfm %0,%1,%2" : "=r"(r) : "r"(ra), "r"(rb)); break;
    case DFMA:  __asm__ volatile ("dfma %0,%1,%2" : "+r"(rt) : "r"(ra), "r"(rb)); r = rt; break;
    case DFMS:  __asm__ volatile ("dfms %0,%1,%2" : "+r"(rt) : "r"(ra), "r"(rb)); r = rt; break;
    case DFNMS: __asm__ volatile ("dfnms %0,%1,%2" : "+r"(rt) : "r"(ra), "r"(rb)); r = rt; break;
    case DFNMA: __asm__ volatile ("dfnma %0,%1,%2" : "+r"(rt) : "r"(ra), "r"(rb)); r = rt; break;
    case FESD:  __asm__ volatile ("fesd %0,%1" : "=r"(r) : "r"(ra)); break;
    default:    __asm__ volatile ("frds %0,%1" : "=r"(r) : "r"(ra)); break;
    }
    __asm__ volatile ("fscrrd %0" : "=r"(fpscr));

    d[0] = op;
    d[1] = rm;
    d[2] = (unsigned int)(a >> 32);
    d[3] = (unsigned int)a;
    d[4] = (unsigned int)(b >> 32);
    d[5] = (unsigned int)b;
    d[6] = (unsigned int)(c >> 32);
    d[7] = (unsigned int)c;
    d[8] = spu_extract((vec_uint4)r, 0);
    d[9] = spu_extract((vec_uint4)r, 1);
    d[10] = spu_extract((vec_uint4)r, 2);
    d[11] = spu_extract((vec_uint4)r, 3);
    d[12] = spu_extract((vec_uint4)fpscr, 0);
    d[13] = spu_extract((vec_uint4)fpscr, 1);
    d[14] = spu_extract((vec_uint4)fpscr, 2);
    d[15] = spu_extract((vec_uint4)fpscr, 3);
}

#define ONE 0x3FF0000000000000ull

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    static const u64 DEN[3] = {
        0x000FFFFFFFFFFFFFull, 0x0000000000000001ull, 0x800FFFFFFFFFFFFFull,
    };
    const u64 q = 0x7FF8000000000123ull, q2 = 0x7FF80000000000AAull;
    const u64 s = 0x7FF0000000000456ull;
    const u64 nq = 0xFFF8000000000789ull, nq2 = 0xFFF80000000000BBull;
    const u64 ns = 0xFFF0000000000ABCull;
    const u64 inf = 0x7FF0000000000000ull, ninf = 0xFFF0000000000000ull;
    unsigned int i, rm, sent;

    for (i = 0; i < 3; i++) {
        u64 x = DEN[i];
        record(DFA, 0, x, ONE, 0);
        record(DFA, 0, ONE, x, 0);
        record(DFA, 0, x, 0, 0);
        record(DFA, 0, 0, x, 0);
        record(DFS, 0, x, ONE, 0);
        record(DFM, 0, x, ONE, 0);
        record(DFM, 0, ONE, x, 0);
        record(DFMA, 0, x, ONE, ONE);
        record(DFMA, 0, ONE, x, ONE);
        record(DFMA, 0, ONE, ONE, x);
        record(DFNMS, 0, ONE, ONE, x);
        record(DFMS, 0, x, ONE, ONE);
        record(DFNMA, 0, ONE, x, ONE);
    }

    for (rm = 0; rm < 4; rm++) {
        record(DFM, rm, 0x0170000000000000ull, 0x3D70000000000000ull, 0);
        record(DFM, rm, 0x0170000000000001ull, 0x3D78000000000000ull, 0);
        record(DFM, rm, 0x0170000000000000ull, 0x3AF0000000000000ull, 0);
        record(DFS, rm, 0x0018000000000000ull, 0x0010000000000000ull, 0);
    }

    record(DFA, 0, q, ONE, 0);
    record(DFA, 0, ONE, q, 0);
    record(DFA, 0, s, ONE, 0);
    record(DFA, 0, ONE, s, 0);
    record(DFA, 0, s, q, 0);
    record(DFA, 0, q, s, 0);
    record(DFA, 0, q, q2, 0);
    record(DFA, 0, nq, nq2, 0);
    record(DFA, 0, ns, nq, 0);
    record(DFA, 0, nq, ONE, 0);
    record(DFM, 0, q, ONE, 0);
    record(DFM, 0, ONE, s, 0);
    record(DFM, 0, nq, nq2, 0);
    record(DFMA, 0, ONE, ONE, q);
    record(DFMA, 0, ONE, ONE, s);
    record(DFMA, 0, nq, ONE, nq2);
    record(DFMA, 0, s, ONE, q);
    record(DFMA, 0, ONE, q, nq);
    record(DFNMS, 0, ONE, ONE, nq);
    record(DFA, 0, inf, ninf, 0);
    record(DFM, 0, 0, inf, 0);

    record(FESD, 0, 0xFF800001ull << 32, 0, 0);
    record(FESD, 0, 0x7FC12345ull << 32, 0, 0);
    record(FESD, 0, 0x00000001ull << 32, 0, 0);
    record(FESD, 0, 0x807FFFFFull << 32, 0, 0);
    record(FRDS, 0, 0xFFF0000000000001ull, 0, 0);
    record(FRDS, 0, 0x7FF8123456789ABCull, 0, 0);
    record(FRDS, 0, 0x000FFFFFFFFFFFFFull, 0, 0);
    for (rm = 0; rm < 4; rm++) {
        record(FRDS, rm, 0x3733333333333333ull, 0, 0);
        record(FRDS, rm, 0x7E70000000000000ull, 0, 0);
    }

    for (sent = 0; sent < OUT_BYTES; sent += 16384) {
        unsigned int len = OUT_BYTES - sent < 16384 ? OUT_BYTES - sent : 16384;
        mfc_put((unsigned char *)out + sent, (unsigned int)argp + sent, len, DMA_TAG, 0, 0);
    }
    mfc_write_tag_mask(1 << DMA_TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
