/* SPU program for spu_estimate_readout: read the frest and frsqest
 * tables off the SPU.
 *
 * Each estimate writes S, the result exponent, a 13-bit BaseFraction
 * and a 10-bit StepFraction into its result, so the result is the table
 * entry for the input's segment. The program records:
 *
 *   A. Plateau scans. For every one of the 2^23 fractions at a fixed
 *      exponent it runs the estimate and records each fraction where
 *      the result changes, with the new result. Three scans: frest at
 *      exponent 127, frsqest at exponent 127 and frsqest at exponent
 *      128 (the two exponent parities).
 *   B. The exponent map. For both estimates, both signs, all 256
 *      exponents and five fractions, the result.
 *   C. The FPSCR after each estimate of a zero or denormal input, with
 *      the FPSCR cleared before each.
 *
 * Result layout (big-endian), DMA'd to the EA in argp:
 *   A: 3 scans x SCAN_WORDS words. Per scan: u32 transition count, u32
 *      results seen past the cap, u32 0, u32 0, then SCAN_CAP pairs of
 *      (u32 fraction, u32 result). The first pair is fraction 0.
 *   B: [estimate 0 = frest, 1 = frsqest][sign 0, 1][exponent 0..255]
 *      [fraction index 0..4] u32 result.
 *   C: per FPSCR case, 16 bytes of result vector then 16 bytes of
 *      FPSCR: frest of each DBZ_INPUT, then frsqest of each.
 *   Zero padding to a multiple of 128 bytes (27008 in all).
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define DMA_TAG    0
#define SCAN_CAP   256
#define SCAN_WORDS (4 + 2 * SCAN_CAP)
#define SCANS      3
#define FRACTIONS  5
#define DBZ_INPUTS 5

#define A_WORDS (SCANS * SCAN_WORDS)
#define B_WORDS (2 * 2 * 256 * FRACTIONS)
#define C_WORDS (2 * DBZ_INPUTS * 8)
#define TOTAL_WORDS (A_WORDS + B_WORDS + C_WORDS)
#define CHUNK 16384

static const unsigned int FRACTION[FRACTIONS] = {
    0x000000, 0x000001, 0x0FF53C, 0x0FF53D, 0x7FFFFF,
};

static const unsigned int DBZ_INPUT[DBZ_INPUTS] = {
    0x00000000, 0x80000000, 0x00000001, 0x000FF53C, 0x007FFFFF,
};

static unsigned int out[(TOTAL_WORDS + 31) & ~31u] __attribute__((aligned(128)));

/* spu_re and spu_rsqrte compile to the estimate followed by fi, so the
 * bare instructions are issued by hand. */
static vec_uint4 frest_v(vec_uint4 v)
{
    vec_uint4 r;
    __asm__ volatile ("frest %0,%1" : "=r"(r) : "r"(v));
    return r;
}

static vec_uint4 frsqest_v(vec_uint4 v)
{
    vec_uint4 r;
    __asm__ volatile ("frsqest %0,%1" : "=r"(r) : "r"(v));
    return r;
}

static unsigned int frest1(unsigned int x)
{
    return spu_extract(frest_v(spu_splats(x)), 0);
}

static unsigned int frsqest1(unsigned int x)
{
    return spu_extract(frsqest_v(spu_splats(x)), 0);
}

static void scan(unsigned int *dst, int sqrt, unsigned int exponent)
{
    unsigned int count = 0, past = 0, last = 0, f;
    for (f = 0; f < (1u << 23); f++) {
        unsigned int x = (exponent << 23) | f;
        unsigned int r = sqrt ? frsqest1(x) : frest1(x);
        if (f == 0 || r != last) {
            if (count < SCAN_CAP) {
                dst[4 + 2 * count] = f;
                dst[5 + 2 * count] = r;
                count++;
            } else {
                past++;
            }
            last = r;
        }
    }
    dst[0] = count;
    dst[1] = past;
    dst[2] = 0;
    dst[3] = 0;
}

static void fpscr_case(unsigned int *dst, int sqrt, unsigned int x)
{
    vec_uint4 zero = spu_splats(0u);
    vec_uint4 v = spu_splats(x);
    vec_uint4 r, fpscr;
    spu_mtfpscr(zero);
    r = sqrt ? frsqest_v(v) : frest_v(v);
    fpscr = spu_mffpscr();
    dst[0] = spu_extract(r, 0);
    dst[1] = spu_extract(r, 1);
    dst[2] = spu_extract(r, 2);
    dst[3] = spu_extract(r, 3);
    dst[4] = spu_extract(fpscr, 0);
    dst[5] = spu_extract(fpscr, 1);
    dst[6] = spu_extract(fpscr, 2);
    dst[7] = spu_extract(fpscr, 3);
}

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    unsigned int *a = out;
    unsigned int *b = out + A_WORDS;
    unsigned int *c = b + B_WORDS;
    unsigned int est, sign, e, k, sent;

    scan(a, 0, 127);
    scan(a + SCAN_WORDS, 1, 127);
    scan(a + 2 * SCAN_WORDS, 1, 128);

    for (est = 0; est < 2; est++)
        for (sign = 0; sign < 2; sign++)
            for (e = 0; e < 256; e++)
                for (k = 0; k < FRACTIONS; k++) {
                    unsigned int x = (sign << 31) | (e << 23) | FRACTION[k];
                    unsigned int i = ((est * 2 + sign) * 256 + e) * FRACTIONS + k;
                    b[i] = est ? frsqest1(x) : frest1(x);
                }

    for (est = 0; est < 2; est++)
        for (k = 0; k < DBZ_INPUTS; k++)
            fpscr_case(c + (est * DBZ_INPUTS + k) * 8, est, DBZ_INPUT[k]);

    for (sent = 0; sent < sizeof(out); sent += CHUNK) {
        unsigned int n = sizeof(out) - sent < CHUNK ? sizeof(out) - sent : CHUNK;
        mfc_put((unsigned char *)out + sent, (unsigned int)argp + sent, n, DMA_TAG, 0, 0);
    }
    mfc_write_tag_mask(1 << DMA_TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
