/* PPU program: the edges of the four VMX estimates that the readout
 * leaves open, each series under VSCR[NJ] = 0 and then NJ = 1. Every
 * series is SERIES_WORDS words; inputs are made from the index alone:
 *
 *   0  vrefp     biased exponent 252: 0x7E000000 | k << 12 | (lcg & 0xFFF)
 *   1  vrefp     biased exponent 253: 0x7E800000 | k << 12 | (lcg & 0xFFF)
 *   2  vrefp     biased exponent 254: 0x7F000000 | k << 12 | (lcg & 0xFFF)
 *   3  vrefp     0xFF000000 | k << 12 | (lcg & 0xFFF), series 2 negated
 *   4  vexptefp  -126 - k / 64 (x from -126 down to about -158)
 *   5  vexptefp  -126 - (k + (lcg & 0xFFFF) / 65536) / 64
 *   6  vexptefp  127 + k / 1024 (x from 127 up to 129)
 *   7  vexptefp  k - 1024 (the integers from -1024 to 1023)
 *
 * Then the negative exponent maps, EXPONENT_WORDS each, op by op
 * (vrefp, vrsqrtefp, vexptefp, vlogefp): word 2e is 0x80000000 | e << 23,
 * word 2e + 1 is 0x80000000 | e << 23 | 0x7FFFFF.
 *
 * The frame holds the NJ = 0 results of every series and map, then the
 * NJ = 1 results in the same order. The LCG is
 * x = x * 1664525 + 1013904223 from 777, one step per random word.
 */

#include <altivec.h>
#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SERIES 8
#define SERIES_WORDS 2048
#define EXPONENT_WORDS 512
#define HALF_WORDS (SERIES * SERIES_WORDS + 4 * EXPONENT_WORDS)

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

static unsigned int inputs[HALF_WORDS] __attribute__((aligned(16)));
static unsigned int results[2 * HALF_WORDS] __attribute__((aligned(16)));

static unsigned int lcg_state = 777;

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

static void run(int op, unsigned int base, unsigned int n, unsigned int *dst)
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
        vec_st((vector unsigned int)d, 0, &dst[base + i]);
    }
}

int main(void)
{
    unsigned int k, e, op, nj, base;

    for (k = 0; k < SERIES_WORDS; k++) {
        inputs[0 * SERIES_WORDS + k] = 0x7E000000u | k << 12 | (lcg() & 0xFFFu);
        inputs[1 * SERIES_WORDS + k] = 0x7E800000u | k << 12 | (lcg() & 0xFFFu);
        inputs[2 * SERIES_WORDS + k] = 0x7F000000u | k << 12 | (lcg() & 0xFFFu);
        inputs[3 * SERIES_WORDS + k] = 0xFF000000u | k << 12 | (lcg() & 0xFFFu);
        inputs[4 * SERIES_WORDS + k] = float_bits(-126.0f - (float)k / 64.0f);
        inputs[5 * SERIES_WORDS + k] =
            float_bits(-126.0f - ((float)k + (float)(lcg() & 0xFFFFu) / 65536.0f) / 64.0f);
        inputs[6 * SERIES_WORDS + k] = float_bits(127.0f + (float)k / 1024.0f);
        inputs[7 * SERIES_WORDS + k] = float_bits((float)((int)k - 1024));
    }
    base = SERIES * SERIES_WORDS;
    for (op = 0; op < 4; op++)
        for (e = 0; e < 256; e++) {
            unsigned int at = base + op * EXPONENT_WORDS + 2 * e;
            inputs[at] = 0x80000000u | e << 23;
            inputs[at + 1] = 0x80000000u | e << 23 | 0x7FFFFFu;
        }

    for (nj = 0; nj < 2; nj++) {
        unsigned int *dst = results + nj * HALF_WORDS;
        set_vscr(nj << 16);
        run(0, 0, 4 * SERIES_WORDS, dst);
        run(2, 4 * SERIES_WORDS, 4 * SERIES_WORDS, dst);
        for (op = 0; op < 4; op++)
            run(op, base + op * EXPONENT_WORDS, EXPONENT_WORDS, dst);
    }
    set_vscr(0x10000);

    write_frame(results, sizeof(results));
    return 0;
}
