/* SPU program for spu_convert_sweep: cflts, cfltu, csflt and cuflt at
 * every I8 value over 4096 inputs, returned as one digest per opcode
 * and I8, plus every result of cflts and cfltu at I8 0..15 on the
 * first 1024 inputs.
 *
 * Each conversion runs from a two-instruction stub in local store: the
 * conversion word (RT = RA = $3) and `bi $0`. The stub is rewritten
 * for each opcode and I8, and `sync` orders the store before the next
 * fetch. The FPSCR is cleared before each call and read after it.
 *
 * Inputs, in order: k = 0..1023 is sign (k >> 9), biased exponent
 * ((k >> 1) & 0xff) and fraction 0 for even k, 0x7FFFFF for odd k;
 * k = 1024..4095 is xorshift32 from 0x2545F491 (one step per input).
 *
 * Digest per (opcode, I8), starting at 0, over the inputs four at a
 * time: h = h * 31 + result for each of the four slots, then for csflt
 * and cuflt h = h * 31 + w for each of the four FPSCR words.
 *
 * Result layout (big-endian words), DMA'd to the EA in argp:
 *   [0, 1024):      digests, index opcode * 256 + I8 (cflts, cfltu,
 *                   csflt, cuflt).
 *   [1024, 33792):  results, index ((opcode * 16 + I8) * 1024 + k) for
 *                   opcode cflts, cfltu and I8 0..15.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define DMA_TAG 0
#define INPUTS 4096
#define RAW_INPUTS 1024
#define OUT_WORDS (1024 + 2 * 16 * RAW_INPUTS)

static const unsigned int OPCODES[4] = { 0x1d8, 0x1d9, 0x1da, 0x1db };

static unsigned int inputs[INPUTS] __attribute__((aligned(16)));
static unsigned int out[OUT_WORDS] __attribute__((aligned(128)));

/* The conversion word, then `bi $0`. */
static unsigned int stub[4] __attribute__((aligned(16))) = {
    0, 0x35000000, 0, 0,
};

typedef vec_uint4 (*conv_fn)(vec_uint4);

static void make_inputs(void)
{
    unsigned int k, state = 0x2545F491u;
    for (k = 0; k < RAW_INPUTS; k++)
        inputs[k] = ((k >> 9) << 31) | (((k >> 1) & 0xff) << 23) | ((k & 1) ? 0x7fffff : 0);
    for (; k < INPUTS; k++) {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        inputs[k] = state;
    }
}

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    conv_fn conv = (conv_fn)(void *)stub;
    unsigned int op, i8, k, lane, sent;

    make_inputs();

    for (op = 0; op < 4; op++)
        for (i8 = 0; i8 < 256; i8++) {
            unsigned int h = 0;
            stub[0] = (OPCODES[op] << 22) | (i8 << 14) | (3 << 7) | 3;
            __asm__ volatile ("sync" ::: "memory");
            for (k = 0; k < INPUTS; k += 4) {
                vec_uint4 x = *(const vec_uint4 *)&inputs[k];
                vec_uint4 r, fpscr;
                spu_mtfpscr(spu_splats(0u));
                r = conv(x);
                fpscr = spu_mffpscr();
                for (lane = 0; lane < 4; lane++) {
                    unsigned int v = spu_extract(r, lane);
                    h = h * 31 + v;
                    if (op < 2 && i8 < 16 && k < RAW_INPUTS)
                        out[1024 + (op * 16 + i8) * RAW_INPUTS + k + lane] = v;
                }
                if (op >= 2)
                    for (lane = 0; lane < 4; lane++)
                        h = h * 31 + spu_extract(fpscr, lane);
            }
            out[op * 256 + i8] = h;
        }

    for (sent = 0; sent < sizeof(out); sent += 16384) {
        unsigned int len = sizeof(out) - sent < 16384 ? sizeof(out) - sent : 16384;
        mfc_put((unsigned char *)out + sent, (unsigned int)argp + sent, len, DMA_TAG, 0, 0);
        mfc_write_tag_mask(1 << DMA_TAG);
        mfc_read_tag_status_all();
    }

    spu_thread_exit(0);
    return 0;
}
