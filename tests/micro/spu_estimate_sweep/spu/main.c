/* SPU program for spu_estimate_sweep: compare a model of frest and
 * frsqest against the instructions over all 2^32 inputs.
 *
 * The model is the one spu_estimate_readout read off the SPU:
 *
 *   frest:   S kept; result exponent 255 for exponent 0, else
 *            253 - e saturating at 0; entry FREST_TABLE[f >> 18].
 *   frsqest: S cleared; result exponent 255 for exponent 0, else
 *            190 - ((e + 1) >> 1); entry FRSQEST_TABLE[(e & 1) << 5
 *            | f >> 18] (even exponents first).
 *
 * An entry is BaseFraction << 10 | StepFraction, the result's low 23
 * bits.
 *
 * A control sweep runs the frest model with entry 5 changed over the
 * 2^23 inputs of exponent 127: it must count exactly 2^18 mismatches,
 * the inputs of that one segment, so a zero count from the full
 * sweeps cannot come from a compare that never fires.
 *
 * Result layout (128 bytes, big-endian), DMA'd to the EA in argp, one
 * 8-word block per sweep (frest, frsqest, control), then 8 zero words:
 *   mismatches hi, mismatches lo, first mismatching input, the
 *   instruction's result there, the model's, inputs swept hi, lo, 0.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define DMA_TAG 0

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

static unsigned int broken_frest[32];

static unsigned int result[32] __attribute__((aligned(128)));

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

static inline vec_uint4 lookup(const unsigned int *table, vec_uint4 idx)
{
    vec_uint4 r = spu_splats(0u);
    r = spu_insert(table[spu_extract(idx, 0)], r, 0);
    r = spu_insert(table[spu_extract(idx, 1)], r, 1);
    r = spu_insert(table[spu_extract(idx, 2)], r, 2);
    r = spu_insert(table[spu_extract(idx, 3)], r, 3);
    return r;
}

static inline vec_uint4 frest_model(const unsigned int *table, vec_uint4 x)
{
    vec_uint4 e = spu_and(spu_rlmask(x, -23), 0xff);
    vec_uint4 idx = spu_and(spu_rlmask(x, -18), 31);
    vec_uint4 re = spu_sub(spu_splats(253u), e);
    re = spu_sel(re, spu_splats(0u), spu_cmpgt(e, 253u));
    re = spu_sel(re, spu_splats(255u), spu_cmpeq(e, 0u));
    return spu_or(spu_or(spu_and(x, 0x80000000u), spu_sl(re, 23)), lookup(table, idx));
}

static inline vec_uint4 frsqest_model(vec_uint4 x)
{
    vec_uint4 e = spu_and(spu_rlmask(x, -23), 0xff);
    vec_uint4 idx = spu_or(spu_sl(spu_and(e, 1u), 5), spu_and(spu_rlmask(x, -18), 31));
    vec_uint4 re = spu_sub(spu_splats(190u), spu_rlmask(spu_add(e, 1u), -1));
    re = spu_sel(re, spu_splats(255u), spu_cmpeq(e, 0u));
    return spu_or(spu_sl(re, 23), lookup(FRSQEST_TABLE, idx));
}

static const unsigned int LANES[4] __attribute__((aligned(16))) = { 0, 1, 2, 3 };

/* Sweep `vectors` groups of four consecutive inputs from `start`. */
static void sweep(unsigned int *dst, int sqrt, const unsigned int *table,
                  unsigned int start, unsigned int vectors)
{
    vec_uint4 x = spu_add(spu_splats(start), *(const vec_uint4 *)LANES);
    vec_uint4 four = spu_splats(4u);
    unsigned long long mismatches = 0;
    unsigned int first_x = 0, first_hw = 0, first_model = 0, found = 0, i;

    for (i = 0; i < vectors; i++) {
        vec_uint4 hw = sqrt ? frsqest_hw(x) : frest_hw(x);
        vec_uint4 model = sqrt ? frsqest_model(x) : frest_model(table, x);
        unsigned int same = spu_extract(spu_gather(spu_cmpeq(hw, model)), 0);
        if (__builtin_expect(same != 0xF, 0)) {
            unsigned int lane;
            for (lane = 0; lane < 4; lane++) {
                if (!(same & (8u >> lane))) {
                    mismatches++;
                    if (!found) {
                        found = 1;
                        first_x = spu_extract(x, lane);
                        first_hw = spu_extract(hw, lane);
                        first_model = spu_extract(model, lane);
                    }
                }
            }
        }
        x = spu_add(x, four);
    }
    dst[0] = (unsigned int)(mismatches >> 32);
    dst[1] = (unsigned int)mismatches;
    dst[2] = first_x;
    dst[3] = first_hw;
    dst[4] = first_model;
    dst[5] = (unsigned int)(((unsigned long long)vectors * 4) >> 32);
    dst[6] = (unsigned int)((unsigned long long)vectors * 4);
    dst[7] = 0;
}

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    unsigned int k;

    for (k = 0; k < 32; k++)
        broken_frest[k] = FREST_TABLE[k];
    broken_frest[5] ^= 1;

    sweep(result, 0, FREST_TABLE, 0, 1u << 30);
    sweep(result + 8, 1, FRSQEST_TABLE, 0, 1u << 30);
    sweep(result + 16, 0, broken_frest, 127u << 23, 1u << 21);

    mfc_put(result, (unsigned int)argp, sizeof(result), DMA_TAG, 0, 0);
    mfc_write_tag_mask(1 << DMA_TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
