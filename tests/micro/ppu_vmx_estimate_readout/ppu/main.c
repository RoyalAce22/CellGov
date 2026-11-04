/* PPU program: read back the four VMX estimates, vrefp, vrsqrtefp,
 * vexptefp and vlogefp, over a grid, random inputs and every exponent,
 * and report every result word as one CGOV frame.
 *
 * Every series is SERIES_WORDS words. Inputs are made from the index
 * alone, so the frame holds only results:
 *
 *   0  vrefp     grid: 0x3F800000 | k << 11
 *   1  vrefp     random: 0x3F800000 | (lcg & 0x7FFFFF)
 *   2  vrsqrtefp grid, biased exponent 127: 0x3F800000 | k << 11
 *   3  vrsqrtefp grid, biased exponent 128: 0x40000000 | k << 11
 *   4  vrsqrtefp random: (127 + (k & 1)) << 23 | (lcg & 0x7FFFFF)
 *   5  vexptefp  grid: k / 4096
 *   6  vexptefp  grid: -k / 4096
 *   7  vexptefp  random: (int32)(lcg) >> 8, divided by 2^23
 *   8  vlogefp   grid: 0x3F800000 | k << 11
 *   9  vlogefp   random: (100 + lcg % 55) << 23 | (lcg' & 0x7FFFFF)
 *
 * Then the exponent maps, EXPONENT_WORDS each, under VSCR[NJ] = 0 and
 * then NJ = 1, op by op in the order above: word 2e is e << 23, word
 * 2e + 1 is e << 23 | 0x7FFFFF.
 *
 * The series run under VSCR = 0 (NJ = 0). The LCG is
 * x = x * 1664525 + 1013904223 from 12345, one step per random word;
 * series 9 takes two steps per word.
 */

#include <altivec.h>
#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SERIES 10
#define SERIES_WORDS 4096
#define EXPONENT_WORDS 512
#define TOTAL_WORDS (SERIES * SERIES_WORDS + 2 * 4 * EXPONENT_WORDS)

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_frame(const void *payload, unsigned int len)
{
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, payload, len, &written);
    CGOV_OUT_FILE_WRITE(payload, len);
}

static unsigned int inputs[TOTAL_WORDS] __attribute__((aligned(16)));
static unsigned int results[TOTAL_WORDS] __attribute__((aligned(16)));

static unsigned int lcg_state = 12345;

static unsigned int lcg(void)
{
    lcg_state = lcg_state * 1664525u + 1013904223u;
    return lcg_state;
}

static unsigned int float_bits(float f)
{
    union { float f; unsigned int u; } v;
    v.f = f;
    return v.u;
}

static void set_vscr(unsigned int value)
{
    vector unsigned int v = { value, value, value, value };
    vec_mtvscr(v);
}

/* Run `op` over n words of inputs into results, four at a time. */
static void run(int op, unsigned int base, unsigned int n)
{
    unsigned int i;
    for (i = 0; i < n; i += 4) {
        vector float b = (vector float)vec_ld(0, &inputs[base + i]);
        vector float d;
        switch (op) {
        case 0: d = vec_re(b); break;
        case 1: d = vec_rsqrte(b); break;
        case 2: d = vec_expte(b); break;
        default: d = vec_loge(b); break;
        }
        vec_st((vector unsigned int)d, 0, &results[base + i]);
    }
}

int main(void)
{
    unsigned int k, e, op, nj, base;

    for (k = 0; k < SERIES_WORDS; k++) {
        unsigned int r;
        inputs[0 * SERIES_WORDS + k] = 0x3F800000u | k << 11;
        inputs[1 * SERIES_WORDS + k] = 0x3F800000u | (lcg() & 0x7FFFFFu);
        inputs[2 * SERIES_WORDS + k] = 0x3F800000u | k << 11;
        inputs[3 * SERIES_WORDS + k] = 0x40000000u | k << 11;
        inputs[4 * SERIES_WORDS + k] = (127u + (k & 1)) << 23 | (lcg() & 0x7FFFFFu);
        inputs[5 * SERIES_WORDS + k] = float_bits((float)k / 4096.0f);
        inputs[6 * SERIES_WORDS + k] = float_bits(-(float)k / 4096.0f);
        inputs[7 * SERIES_WORDS + k] = float_bits((float)((int)lcg() >> 8) / 8388608.0f);
        r = lcg();
        inputs[8 * SERIES_WORDS + k] = 0x3F800000u | k << 11;
        inputs[9 * SERIES_WORDS + k] = (100u + r % 55u) << 23 | (lcg() & 0x7FFFFFu);
    }
    base = SERIES * SERIES_WORDS;
    for (nj = 0; nj < 2; nj++)
        for (op = 0; op < 4; op++)
            for (e = 0; e < 256; e++) {
                unsigned int at = base + (nj * 4 + op) * EXPONENT_WORDS + 2 * e;
                inputs[at] = e << 23;
                inputs[at + 1] = e << 23 | 0x7FFFFFu;
            }

    set_vscr(0);
    run(0, 0 * SERIES_WORDS, 2 * SERIES_WORDS);
    run(1, 2 * SERIES_WORDS, 3 * SERIES_WORDS);
    run(2, 5 * SERIES_WORDS, 3 * SERIES_WORDS);
    run(3, 8 * SERIES_WORDS, 2 * SERIES_WORDS);
    for (nj = 0; nj < 2; nj++) {
        set_vscr(nj << 16);
        for (op = 0; op < 4; op++)
            run(op, base + (nj * 4 + op) * EXPONENT_WORDS, EXPONENT_WORDS);
    }
    set_vscr(0x10000);

    write_frame(results, sizeof(results));
    return 0;
}
