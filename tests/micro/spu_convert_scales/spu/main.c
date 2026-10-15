/* SPU program for spu_convert_scales: cflts, cfltu, csflt and cuflt at
 * every I8 value, the 128 the architecture defines and the 128 it
 * leaves undefined, on 16 inputs.
 *
 * The conversions themselves are in the generated assembly
 * (gen_convert.py); this file loads the inputs and returns the buffer.
 *
 * Result layout (big-endian), DMA'd to the EA in argp:
 *   [0, 65536):     per (opcode cflts, cfltu, csflt, cuflt; I8 0..255;
 *                   input vector 0..3) the 16-byte result.
 *   [65536, 98304): per (opcode csflt, cuflt; I8; input vector) the
 *                   FPSCR after the conversion, cleared before it.
 * The 16 inputs are INPUTS below, four per vector in order.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define DMA_TAG 0
#define OUT_BYTES 98304

static const unsigned int INPUTS[16] __attribute__((aligned(16))) = {
    0x00000000, 0x80000000, 0x3f800000, 0xbf800000,
    0x3f000000, 0x3fc00000, 0x4f000000, 0xcf000000,
    0x4f800000, 0x7f7fffff, 0x7fffffff, 0xffffffff,
    0x00000001, 0x00800000, 0x0000007f, 0x12345678,
};

unsigned int out[OUT_BYTES / 4] __attribute__((aligned(128)));

void run_conversions(vec_uint4 a, vec_uint4 b, vec_uint4 c, vec_uint4 d);

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    const vec_uint4 *in = (const vec_uint4 *)INPUTS;
    unsigned int sent;

    run_conversions(in[0], in[1], in[2], in[3]);

    for (sent = 0; sent < OUT_BYTES; sent += 16384) {
        mfc_put((unsigned char *)out + sent, (unsigned int)argp + sent, 16384, DMA_TAG, 0, 0);
        /* At most 16 commands are queued; wait every 4 to stay well
         * inside that. */
        if ((sent / 16384) % 4 == 3) {
            mfc_write_tag_mask(1 << DMA_TAG);
            mfc_read_tag_status_all();
        }
    }
    mfc_write_tag_mask(1 << DMA_TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
