/* SPU program for spu_fi_datapath: the fi datapath and its flags.
 *
 * fi computes (1.BaseFraction - 0.000StepFraction x Y) x 2^(e-127),
 * with BaseFraction, StepFraction, e and the sign from RB and Y from
 * RA's low 19 bits. The model compared here evaluates that exactly and
 * truncates to 24 significant bits.
 *
 *   A. Every table entry frest and frsqest return (32 + 64), then four
 *      extremes (base 0 / 0x1FFF x step 0 / 0x3FF), each against all
 *      2^19 values of Y. RB has exponent 127; the sign alternates by
 *      slot. RA's upper 13 bits are scrambled, so a model that reads
 *      them would differ. The FPSCR is cleared before each entry and
 *      read after its sweep.
 *   B. 2^22 pseudo-random (RA, RB) pairs. Pairs whose RB exponent is 0
 *      or 1 (where the result can leave the normal range) are counted
 *      and skipped.
 *   C. Flag vectors: RB exponent 0, 1, 254, 255 x RA exponent 0, 255 x
 *      two entries (one whose value stays at or above 1, one that drops
 *      below 1 and renormalizes). The FPSCR is cleared before each.
 *   D. The reciprocal sequence (frest; fi; fnms; fma) and the
 *      reciprocal-square-root sequence (frsqest; and; fi; fm; fm; fnms;
 *      fma) over all 2^23 fractions at exponents 127 and 128. Per sweep:
 *      a digest of every y2, the FPSCR after the sweep, and the y2 of
 *      every fraction that is a multiple of 0x8000.
 *
 * Result layout (big-endian words), DMA'd to the EA in argp:
 *   A: 100 x 16 words: mismatches, first mismatching RA, its RB, fi's
 *      result, the model's, 3 zero words, the FPSCR (4 words), 4 zero
 *      words.
 *   B: 48 words: mismatches, skipped, pairs compared, 1 zero word, then
 *      up to 8 mismatches as (RA, RB, fi, model), then 12 zero words.
 *   C: 16 x 8 words: RA, RB, fi's result (slot 0), 1 zero word, the
 *      FPSCR (4 words). Case k = (rb_exp_index * 2 + ra_exp_index) * 2
 *      + entry.
 *   D: 4 x 264 words (recip 127, recip 128, rsqrt 127, rsqrt 128):
 *      digest, 3 zero words, the FPSCR (4 words), 256 sampled y2.
 *      The digest is h = h * 31 + y2 over the fractions in order,
 *      starting at 0.
 *   Zero padding to 11392 bytes.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define DMA_TAG 0

#define A_ENTRIES 100
#define A_WORDS (A_ENTRIES * 16)
#define B_WORDS 48
#define C_CASES 16
#define C_WORDS (C_CASES * 8)
#define D_SWEEPS 4
#define D_WORDS (D_SWEEPS * 264)
#define TOTAL_WORDS (A_WORDS + B_WORDS + C_WORDS + D_WORDS)

static const unsigned int FREST_TABLE[32] = {
    0x7ffbe0, 0x7f87a6, 0x70ef72, 0x708b40, 0x638b12, 0x633aea, 0x5792c4, 0x574aa0,
    0x4cca7e, 0x4c9262, 0x430a44, 0x42d62a, 0x3a2e12, 0x39fdfa, 0x3215e4, 0x31f1d2,
    0x2aa9be, 0x2a85ac, 0x23d59a, 0x23bd8e, 0x1d8576, 0x1d8576, 0x17ad5a, 0x17ad5a,
    0x124543, 0x124543, 0x0d392d, 0x0d392d, 0x08851a, 0x08851a, 0x041d07, 0x041d07,
};

static const unsigned int FRSQEST_TABLE[64] = {
    0x350160, 0x34e954, 0x2f993d, 0x2f993d, 0x2aa523, 0x2aa523, 0x26190d, 0x26190d,
    0x21e4f9, 0x21e4f9, 0x1e00e9, 0x1e00e9, 0x1a5cd9, 0x1a5cd9, 0x16f8cb, 0x16f8cb,
    0x13ccc0, 0x13ccc0, 0x10ccb3, 0x10ccb3, 0x0e00aa, 0x0e00aa, 0x0b58a1, 0x0b58a1,
    0x08d498, 0x08d498, 0x067491, 0x067491, 0x043089, 0x043089, 0x020c83, 0x020c83,
    0x7ffdf4, 0x7fd1de, 0x7859c8, 0x783dba, 0x71559c, 0x71559c, 0x6ae57c, 0x6ae57c,
    0x64f561, 0x64f561, 0x5f7149, 0x5f7149, 0x5a4d33, 0x5a4d33, 0x55811f, 0x55811f,
    0x51050f, 0x51050f, 0x4cc8fe, 0x4cc8fe, 0x48d0f0, 0x48d0f0, 0x4510e4, 0x4510e4,
    0x4180d7, 0x4180d7, 0x3e24cc, 0x3e24cc, 0x3af4c3, 0x3af4c3, 0x37e8ba, 0x37e8ba,
};

/* base << 10 | step for the four extremes. */
static const unsigned int EXTREMES[4] = {
    0x000000, 0x0003ff, 0x7ffc00, 0x7fffff,
};

static const unsigned int C_RB_EXP[4] = { 0, 1, 254, 255 };
static const unsigned int C_RA_EXP[2] = { 0, 255 };
/* One entry whose value stays >= 1 at Y = 0x7FFFF, one that drops
 * below 1 and renormalizes. */
static const unsigned int C_ENTRY[2] = { 0x7ffc00, 0x0003ff };

static const unsigned int LANES[4] __attribute__((aligned(16))) = { 0, 1, 2, 3 };
static const unsigned int SIGNS[4] __attribute__((aligned(16))) = {
    0, 0x80000000, 0, 0x80000000,
};

static unsigned int out[(TOTAL_WORDS + 31) & ~31u] __attribute__((aligned(128)));

static inline vec_uint4 fi_hw(vec_uint4 ra, vec_uint4 rb)
{
    vec_uint4 r;
    __asm__ ("fi %0,%1,%2" : "=r"(r) : "r"(ra), "r"(rb));
    return r;
}

static inline vec_uint4 frest_hw(vec_uint4 v)
{
    vec_uint4 r;
    __asm__ ("frest %0,%1" : "=r"(r) : "r"(v));
    return r;
}

static inline vec_uint4 frsqest_hw(vec_uint4 v)
{
    vec_uint4 r;
    __asm__ ("frsqest %0,%1" : "=r"(r) : "r"(v));
    return r;
}

/* The model, valid for an RB exponent of 2 or more. */
static unsigned int fi_model(unsigned int ra, unsigned int rb)
{
    unsigned int e = (rb >> 23) & 0xff;
    unsigned long long base = (rb >> 10) & 0x1fff;
    unsigned long long step = rb & 0x3ff;
    unsigned long long y = ra & 0x7ffff;
    unsigned long long v = ((8192 + base) << 19) - step * y;
    unsigned int frac;
    if (v >> 32) {
        frac = (unsigned int)(v >> 9) & 0x7fffff;
    } else {
        frac = (unsigned int)(v >> 8) & 0x7fffff;
        e -= 1;
    }
    return (rb & 0x80000000u) | (e << 23) | frac;
}

static void store_fpscr(unsigned int *dst, vec_uint4 fpscr)
{
    dst[0] = spu_extract(fpscr, 0);
    dst[1] = spu_extract(fpscr, 1);
    dst[2] = spu_extract(fpscr, 2);
    dst[3] = spu_extract(fpscr, 3);
}

static void part_a(unsigned int *dst, unsigned int entry)
{
    vec_uint4 rb = spu_or(spu_splats((127u << 23) | entry), *(const vec_uint4 *)SIGNS);
    vec_uint4 lanes = *(const vec_uint4 *)LANES;
    unsigned int mismatches = 0, y, lane;
    spu_mtfpscr(spu_splats(0u));
    for (y = 0; y < (1u << 19); y += 4) {
        /* Scramble RA's upper 13 bits. */
        unsigned int high = (y * 0x9E3779B9u) & 0xFFF80000u;
        vec_uint4 ra = spu_or(spu_splats(high | y), lanes);
        vec_uint4 hw = fi_hw(ra, rb);
        for (lane = 0; lane < 4; lane++) {
            unsigned int a = spu_extract(ra, lane), b = spu_extract(rb, lane);
            unsigned int h = spu_extract(hw, lane), m = fi_model(a, b);
            if (h != m) {
                if (mismatches == 0) {
                    dst[1] = a;
                    dst[2] = b;
                    dst[3] = h;
                    dst[4] = m;
                }
                mismatches++;
            }
        }
    }
    store_fpscr(dst + 8, spu_mffpscr());
    dst[0] = mismatches;
}

static void part_b(unsigned int *dst)
{
    unsigned int state = 0x2545F491u, mismatches = 0, skipped = 0, compared = 0, i;
    for (i = 0; i < (1u << 22); i++) {
        unsigned int ra, rb, e, h, m;
        state ^= state << 13; state ^= state >> 17; state ^= state << 5;
        ra = state;
        state ^= state << 13; state ^= state >> 17; state ^= state << 5;
        rb = state;
        e = (rb >> 23) & 0xff;
        if (e < 2) {
            skipped++;
            continue;
        }
        h = spu_extract(fi_hw(spu_splats(ra), spu_splats(rb)), 0);
        m = fi_model(ra, rb);
        compared++;
        if (h != m) {
            if (mismatches < 8) {
                dst[4 + mismatches * 4] = ra;
                dst[5 + mismatches * 4] = rb;
                dst[6 + mismatches * 4] = h;
                dst[7 + mismatches * 4] = m;
            }
            mismatches++;
        }
    }
    dst[0] = mismatches;
    dst[1] = skipped;
    dst[2] = compared;
}

static void part_c(unsigned int *dst)
{
    unsigned int i, j, k;
    for (i = 0; i < 4; i++)
        for (j = 0; j < 2; j++)
            for (k = 0; k < 2; k++) {
                unsigned int *d = dst + ((i * 2 + j) * 2 + k) * 8;
                unsigned int ra = (C_RA_EXP[j] << 23) | 0x7ffff;
                unsigned int rb = (C_RB_EXP[i] << 23) | C_ENTRY[k];
                vec_uint4 r;
                spu_mtfpscr(spu_splats(0u));
                r = fi_hw(spu_splats(ra), spu_splats(rb));
                store_fpscr(d + 4, spu_mffpscr());
                d[0] = ra;
                d[1] = rb;
                d[2] = spu_extract(r, 0);
            }
}

static void part_d(unsigned int *dst, int sqrt, unsigned int exponent)
{
    vec_float4 one = spu_splats(1.0f), half = spu_splats(0.5f);
    vec_uint4 mask = spu_splats(0x7fffffffu);
    vec_uint4 lanes = *(const vec_uint4 *)LANES;
    unsigned int h = 0, f, lane;
    spu_mtfpscr(spu_splats(0u));
    for (f = 0; f < (1u << 23); f += 4) {
        vec_uint4 xu = spu_or(spu_splats((exponent << 23) | f), lanes);
        vec_float4 x = (vec_float4)xu, y2;
        if (sqrt) {
            vec_uint4 y0 = frsqest_hw(xu);
            vec_uint4 axu = spu_and(xu, mask);
            vec_float4 ax = (vec_float4)axu;
            vec_float4 y1 = (vec_float4)fi_hw(axu, y0);
            vec_float4 t1 = spu_mul(ax, y1);
            vec_float4 t2 = spu_mul(y1, half);
            t1 = spu_nmsub(t1, y1, one);
            y2 = spu_madd(t1, t2, y1);
        } else {
            vec_uint4 y0 = frest_hw(xu);
            vec_float4 y1 = (vec_float4)fi_hw(xu, y0);
            vec_float4 t1 = spu_nmsub(x, y1, one);
            y2 = spu_madd(t1, y1, y1);
        }
        for (lane = 0; lane < 4; lane++) {
            unsigned int r = spu_extract((vec_uint4)y2, lane);
            h = h * 31 + r;
            if (((f + lane) & 0x7fff) == 0)
                dst[8 + ((f + lane) >> 15)] = r;
        }
    }
    store_fpscr(dst + 4, spu_mffpscr());
    dst[0] = h;
}

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    unsigned int *a = out;
    unsigned int *b = a + A_WORDS;
    unsigned int *c = b + B_WORDS;
    unsigned int *d = c + C_WORDS;
    unsigned int n = 0, k, sent;

    for (k = 0; k < 32; k++)
        part_a(a + 16 * n++, FREST_TABLE[k]);
    for (k = 0; k < 64; k++)
        part_a(a + 16 * n++, FRSQEST_TABLE[k]);
    for (k = 0; k < 4; k++)
        part_a(a + 16 * n++, EXTREMES[k]);

    part_b(b);
    part_c(c);

    part_d(d, 0, 127);
    part_d(d + 264, 0, 128);
    part_d(d + 2 * 264, 1, 127);
    part_d(d + 3 * 264, 1, 128);

    for (sent = 0; sent < sizeof(out); sent += 16384) {
        unsigned int len = sizeof(out) - sent < 16384 ? sizeof(out) - sent : 16384;
        mfc_put((unsigned char *)out + sent, (unsigned int)argp + sent, len, DMA_TAG, 0, 0);
    }
    mfc_write_tag_mask(1 << DMA_TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
