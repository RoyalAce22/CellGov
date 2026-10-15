/* SPU program for spu_float_truncation: where single-precision
 * arithmetic truncates, the overflow rule at the top of the range, and
 * when the FPSCR sees a flag.
 *
 * Every case clears the FPSCR, runs one instruction on four equal
 * slots, and records (A, B, C, result of slot 0, FPSCR word 0). FPSCR
 * word 0 holds slot 0's flags (OVF 4, UNF 2, DIFF 1).
 *
 *   1. Opposite-sign sums. For A in { 1.0, 0x3FFFFFFF }, D = 1..60 and
 *      F in { 2^-23, 1 - 2^-23 }, with Y = 2^-D x (1 + F): fa A,-Y then
 *      fs A,Y. 480 cases, in the order (A, D, F, op).
 *   2. Fused forms. A = B = 1 + 2^-23, so the product is
 *      1 + 2^-22 + 2^-46. For D = 1..60, C = 2^-D x (1 + 2^-23) (far
 *      below) and C = 2^D (far above), each with sign + and -: fma
 *      (A*B+C), fms (A*B-C), fnms (C-A*B). 720 cases, in the order
 *      (op, D, below then above, + then -).
 *   3. Overflow at the top of the range, Smax = 0x7FFFFFFF:
 *      fm Smax, 1 + k x 2^-23 for k = 0..15, then fa Smax, Y with
 *      Y = E << 23 for biased exponents E = 200..255. 72 cases.
 *   4. FPSCR, 28 words:
 *      [0..4)   fscrrd after fscrwr of all ones;
 *      [4..8)   fscrrd after an overflowing fa followed by an fa that
 *               raises nothing;
 *      [8..28)  5 x 4 words: fscrrd after an overflowing fa followed
 *               by 0, 1, 2, 3 and 4 nops.
 *
 * Result layout (big-endian words), DMA'd to the EA in argp:
 *   cases 1-3 as 5 words each (1272 cases, 6360 words), then the 28
 *   FPSCR words, then zero padding to 25600 bytes.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define DMA_TAG 0
#define CASES (480 + 720 + 72)
#define CASE_WORDS (CASES * 5)
#define FPSCR_WORDS 28
#define TOTAL_WORDS (CASE_WORDS + FPSCR_WORDS)

#define SMAX 0x7fffffffu

static unsigned int out[6400] __attribute__((aligned(128)));
static unsigned int n_cases;

enum op { FA, FS, FM, FMA, FMS, FNMS };

/* The asm hides the value from the compiler, so no operation on it is
 * folded at compile time under the host's IEEE rules. */
static inline vec_float4 f4(unsigned int x)
{
    vec_uint4 v = spu_splats(x);
    __asm__ ("" : "+r"(v));
    return (vec_float4)v;
}

static void __attribute__((noinline)) record(enum op op, unsigned int a, unsigned int b, unsigned int c)
{
    unsigned int *d = out + n_cases++ * 5;
    vec_float4 r;
    vec_uint4 fpscr;
    spu_mtfpscr(spu_splats(0u));
    switch (op) {
    case FA:   r = spu_add(f4(a), f4(b)); break;
    case FS:   r = spu_sub(f4(a), f4(b)); break;
    case FM:   r = spu_mul(f4(a), f4(b)); break;
    case FMA:  r = spu_madd(f4(a), f4(b), f4(c)); break;
    case FMS:  r = spu_msub(f4(a), f4(b), f4(c)); break;
    default:   r = spu_nmsub(f4(a), f4(b), f4(c)); break;
    }
    fpscr = spu_mffpscr();
    d[0] = a;
    d[1] = b;
    d[2] = c;
    d[3] = spu_extract((vec_uint4)r, 0);
    d[4] = spu_extract(fpscr, 0);
}

/* 2^-d x (1 + f x 2^-23) for d <= 126. */
static unsigned int small(unsigned int d, unsigned int f)
{
    return ((127 - d) << 23) | f;
}

#define OVERFLOW_THEN_READ(nops, dst)                                   \
    do {                                                                \
        vec_float4 big = f4(SMAX), sum;                                 \
        vec_uint4 fpscr;                                                \
        spu_mtfpscr(spu_splats(0u));                                    \
        __asm__ volatile ("fa %0,%2,%2\n\t" nops "fscrrd %1"            \
                          : "=&r"(sum), "=r"(fpscr) : "r"(big));        \
        (dst)[0] = spu_extract(fpscr, 0);                               \
        (dst)[1] = spu_extract(fpscr, 1);                               \
        (dst)[2] = spu_extract(fpscr, 2);                               \
        (dst)[3] = spu_extract(fpscr, 3);                               \
    } while (0)

static void fpscr_block(unsigned int *dst)
{
    vec_uint4 fpscr;
    vec_float4 x;

    spu_mtfpscr(spu_splats(0xffffffffu));
    fpscr = spu_mffpscr();
    dst[0] = spu_extract(fpscr, 0);
    dst[1] = spu_extract(fpscr, 1);
    dst[2] = spu_extract(fpscr, 2);
    dst[3] = spu_extract(fpscr, 3);

    /* One asm block, so neither fa can be dropped or moved past the
     * fscrrd. */
    spu_mtfpscr(spu_splats(0u));
    __asm__ volatile ("fa %0,%2,%2\n\tfa %0,%3,%4\n\tfscrrd %1"
                      : "=&r"(x), "=r"(fpscr)
                      : "r"(f4(SMAX)), "r"(f4(0x3f800000u)), "r"(f4(0u)));
    dst[4] = spu_extract(fpscr, 0);
    dst[5] = spu_extract(fpscr, 1);
    dst[6] = spu_extract(fpscr, 2);
    dst[7] = spu_extract(fpscr, 3);

    OVERFLOW_THEN_READ("", dst + 8);
    OVERFLOW_THEN_READ("nop\n\t", dst + 12);
    OVERFLOW_THEN_READ("nop\n\tnop\n\t", dst + 16);
    OVERFLOW_THEN_READ("nop\n\tnop\n\tnop\n\t", dst + 20);
    OVERFLOW_THEN_READ("nop\n\tnop\n\tnop\n\tnop\n\t", dst + 24);
}

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    static const unsigned int A1[2] = { 0x3f800000u, 0x3fffffffu };
    static const unsigned int F1[2] = { 1u, 0x7fffffu };
    unsigned int i, d, j, s, k, e, sent;
    const unsigned int ab = 0x3f800001u;

    for (i = 0; i < 2; i++)
        for (d = 1; d <= 60; d++)
            for (j = 0; j < 2; j++) {
                unsigned int y = small(d, F1[j]);
                record(FA, A1[i], y | 0x80000000u, 0);
                record(FS, A1[i], y, 0);
            }

    for (k = 0; k < 3; k++)
        for (d = 1; d <= 60; d++)
            for (j = 0; j < 2; j++)
                for (s = 0; s < 2; s++) {
                    unsigned int c = j == 0 ? small(d, 1) : (127 + d) << 23;
                    c |= s << 31;
                    record(k == 0 ? FMA : k == 1 ? FMS : FNMS, ab, ab, c);
                }

    for (k = 0; k < 16; k++)
        record(FM, SMAX, 0x3f800000u + k, 0);
    for (e = 200; e <= 255; e++)
        record(FA, SMAX, e << 23, 0);

    fpscr_block(out + CASE_WORDS);

    for (sent = 0; sent < sizeof(out); sent += 16384) {
        unsigned int len = sizeof(out) - sent < 16384 ? sizeof(out) - sent : 16384;
        mfc_put((unsigned char *)out + sent, (unsigned int)argp + sent, len, DMA_TAG, 0, 0);
    }
    mfc_write_tag_mask(1 << DMA_TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
