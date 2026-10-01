/* SPU program for spu_sweep_compare: compare candidate models against
 * the hardware instruction over the whole 32-bit input space, on the
 * SPU, and return only the mismatch counts and the first mismatch.
 *
 * The instruction is `or` with a fixed operand K. The first model is
 * the identity x | K == x + K - (x & K); the second is the same model
 * broken on purpose at exactly one input, BROKEN_INPUT, where its low
 * bit is flipped. A sound harness reports zero mismatches for the first
 * and exactly one, at BROKEN_INPUT, for the second.
 *
 * Result layout (32 bytes, big-endian), DMA'd to the EA in argp:
 *   +0  u32 status (0 = the sweep ran)
 *   +4  u32 mismatches of the identity model
 *   +8  u32 mismatches of the broken model
 *   +12 u32 the broken model's first mismatching input
 *   +16 u32 the hardware's result there
 *   +20 u32 the broken model's result there
 *   +24 u32 inputs swept, high word
 *   +28 u32 inputs swept, low word
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define K            0x5A5A5A5Au
#define BROKEN_INPUT 0x12345678u
#define DMA_TAG      0

static unsigned int result[8] __attribute__((aligned(16)));

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    const vec_uint4 k = spu_splats(K);
    const vec_uint4 broken = spu_splats(BROKEN_INPUT);
    const vec_uint4 one = spu_splats(1u);
    const vec_uint4 step = spu_splats(4u);
    vec_uint4 x = { 0, 1, 2, 3 };
    unsigned int ok_mismatches = 0;
    unsigned int broken_mismatches = 0;
    unsigned long long swept = 0;
    unsigned int i;

    result[3] = result[4] = result[5] = 0;
    /* 2^30 iterations of four lanes: every 32-bit input once. */
    for (i = 0; i < (1u << 30); i++) {
        vec_uint4 hw = spu_or(x, k);
        vec_uint4 model = spu_sub(spu_add(x, k), spu_and(x, k));
        vec_uint4 flip = spu_and(spu_cmpeq(x, broken), one);
        vec_uint4 bad_model = spu_xor(model, flip);
        unsigned int ok_eq = spu_extract(spu_gather(spu_cmpeq(hw, model)), 0);
        unsigned int bad_eq = spu_extract(spu_gather(spu_cmpeq(hw, bad_model)), 0);
        if (ok_eq != 0xF || bad_eq != 0xF) {
            int lane;
            for (lane = 0; lane < 4; lane++) {
                unsigned int bit = 8u >> lane;
                if ((ok_eq & bit) == 0)
                    ok_mismatches++;
                if ((bad_eq & bit) == 0) {
                    if (broken_mismatches == 0) {
                        result[3] = spu_extract(x, lane);
                        result[4] = spu_extract(hw, lane);
                        result[5] = spu_extract(bad_model, lane);
                    }
                    broken_mismatches++;
                }
            }
        }
        x = spu_add(x, step);
        swept += 4;
    }

    result[0] = 0;
    result[1] = ok_mismatches;
    result[2] = broken_mismatches;
    result[6] = (unsigned int)(swept >> 32);
    result[7] = (unsigned int)swept;

    mfc_put(result, (unsigned int)argp, sizeof(result), DMA_TAG, 0, 0);
    mfc_write_tag_mask(1 << DMA_TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
