/* SPU program for spu_fi_zero_operand: fi with an RB whose exponent is
 * 0, and fi results that underflow, for both signs.
 *
 * Each case clears the FPSCR, runs fi with RA in all four slots and RB
 * in all four slots, and records the result and the FPSCR.
 *
 * Result layout (big-endian words), DMA'd to the EA in argp:
 *   CASES x 8 words: RA, RB, fi's result (slot 0), 1 zero word, the
 *   FPSCR (4 words). Then zero padding to a multiple of 128 bytes.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define DMA_TAG 0
#define CASES 16

/* RB values: +0, -0, an exponent-0 RB with a fraction of each sign,
 * the smallest nonzero fraction of each sign, and an exponent-1 RB
 * whose value drops below 1 and underflows, of each sign. */
static const unsigned int RB[8] = {
    0x00000000, 0x80000000, 0x007ffc00, 0x807ffc00,
    0x00000001, 0x80000001, 0x008003ff, 0x808003ff,
};

/* Y = all ones and Y = 0. */
static const unsigned int RA[2] = { 0x0007ffff, 0x00000000 };

static unsigned int out[(CASES * 8 + 31) & ~31u] __attribute__((aligned(128)));

static inline vec_uint4 fi_hw(vec_uint4 ra, vec_uint4 rb)
{
    vec_uint4 r;
    __asm__ volatile ("fi %0,%1,%2" : "=r"(r) : "r"(ra), "r"(rb));
    return r;
}

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    unsigned int i, j;

    for (i = 0; i < 8; i++)
        for (j = 0; j < 2; j++) {
            unsigned int *d = out + (i * 2 + j) * 8;
            vec_uint4 r, fpscr;
            spu_mtfpscr(spu_splats(0u));
            r = fi_hw(spu_splats(RA[j]), spu_splats(RB[i]));
            fpscr = spu_mffpscr();
            d[0] = RA[j];
            d[1] = RB[i];
            d[2] = spu_extract(r, 0);
            d[4] = spu_extract(fpscr, 0);
            d[5] = spu_extract(fpscr, 1);
            d[6] = spu_extract(fpscr, 2);
            d[7] = spu_extract(fpscr, 3);
        }

    mfc_put(out, (unsigned int)argp, sizeof(out), DMA_TAG, 0, 0);
    mfc_write_tag_mask(1 << DMA_TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
