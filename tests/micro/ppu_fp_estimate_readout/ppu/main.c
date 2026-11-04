/* PPU program: read back the PPE's scalar estimates, fres and frsqrte,
 * over a grid, random inputs, every exponent and all four rounding
 * modes, and the FPSCR each special input leaves; report every result
 * doubleword as one CGOV frame.
 *
 * Series 0-6 hold SERIES_WORDS results each, made from the index alone:
 *
 *   0  fres     grid: 0x3FF0000000000000 | k << 40
 *   1  fres     random: 0x3FF0000000000000 | (r & 0xFFFFFFFFFFFFF)
 *   2  frsqrte  grid, biased exponent 1023: 0x3FF0000000000000 | k << 40
 *   3  frsqrte  grid, biased exponent 1024: 0x4000000000000000 | k << 40
 *   4  frsqrte  random: (1023 + (k & 1)) << 52 | (r & 0xFFFFFFFFFFFFF)
 *   5  fres     exponent map: word 2e is e << 52, word 2e + 1 is
 *               e << 52 | 0xFFFFFFFFFFFFF, for e = 0 to 2047
 *   6  frsqrte  exponent map, as series 5
 *
 * where r is two LCG steps, high word first. Series 0-6 run with
 * FPSCR[RN] = 0. Then the rounding series: for RN = 0 to 3 in turn,
 * fres and then frsqrte of RN_WORDS random inputs at biased exponent
 * 1023 (the same inputs for every RN). Then the specials: for each of
 * SPECIALS inputs, fres's result and the FPSCR after it, then
 * frsqrte's, with the FPSCR cleared before each.
 *
 * The LCG is x = x * 1664525 + 1013904223 from 4242.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SERIES 7
#define SERIES_WORDS 4096
#define RN_WORDS 1024
#define SPECIALS 16
#define TOTAL (SERIES * SERIES_WORDS + 4 * 2 * RN_WORDS + SPECIALS * 4)

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

static unsigned long long results[TOTAL] __attribute__((aligned(16)));

static unsigned int lcg_state = 4242;

static unsigned int lcg(void)
{
    lcg_state = lcg_state * 1664525u + 1013904223u;
    return lcg_state;
}

static unsigned long long lcg64(void)
{
    unsigned long long hi = lcg();
    return hi << 32 | lcg();
}

static unsigned long long fres_bits(unsigned long long b)
{
    double in, out;
    memcpy(&in, &b, 8);
    __asm__ volatile("fres %0,%1" : "=f"(out) : "f"(in));
    memcpy(&b, &out, 8);
    return b;
}

static unsigned long long frsqrte_bits(unsigned long long b)
{
    double in, out;
    memcpy(&in, &b, 8);
    __asm__ volatile("frsqrte %0,%1" : "=f"(out) : "f"(in));
    memcpy(&b, &out, 8);
    return b;
}

static unsigned long long fpscr(void)
{
    double f;
    unsigned long long b;
    __asm__ volatile("mffs %0" : "=f"(f));
    memcpy(&b, &f, 8);
    return b;
}

static void clear_fpscr(void)
{
    double zero = 0.0;
    __asm__ volatile("mtfsf 0xff,%0" : : "f"(zero));
}

static void set_rn(unsigned int rn)
{
    switch (rn) {
    case 0: __asm__ volatile("mtfsfi 7,0"); break;
    case 1: __asm__ volatile("mtfsfi 7,1"); break;
    case 2: __asm__ volatile("mtfsfi 7,2"); break;
    default: __asm__ volatile("mtfsfi 7,3"); break;
    }
}

static const unsigned long long SPECIAL_INPUTS[SPECIALS] = {
    0x0000000000000000ULL, 0x8000000000000000ULL, /* +0, -0 */
    0x7FF0000000000000ULL, 0xFFF0000000000000ULL, /* +Inf, -Inf */
    0x7FF8000000000000ULL, 0x7FF4000000000001ULL, /* QNaN, SNaN */
    0x0000000000000001ULL, 0x800FFFFFFFFFFFFFULL, /* denormals */
    0x3FF0000000000000ULL, 0xBFF0000000000000ULL, /* +1, -1 */
    0x47F0000000000000ULL, 0x3800000000000000ULL, /* 2^128, 2^-127 */
    0x7FEFFFFFFFFFFFFFULL, 0x0010000000000000ULL, /* largest, smallest normal */
    0x36A0000000000000ULL, 0x47EFFFFFE0000000ULL, /* 2^-149, the largest single */
};

int main(void)
{
    unsigned long long k, at, e;
    unsigned int rn, i;
    unsigned long long rn_inputs[RN_WORDS];

    clear_fpscr();
    for (k = 0; k < SERIES_WORDS; k++) {
        unsigned long long r1 = lcg64(), r2 = lcg64();
        results[0 * SERIES_WORDS + k] = fres_bits(0x3FF0000000000000ULL | k << 40);
        results[1 * SERIES_WORDS + k] = fres_bits(0x3FF0000000000000ULL | (r1 & 0xFFFFFFFFFFFFFULL));
        results[2 * SERIES_WORDS + k] = frsqrte_bits(0x3FF0000000000000ULL | k << 40);
        results[3 * SERIES_WORDS + k] = frsqrte_bits(0x4000000000000000ULL | k << 40);
        results[4 * SERIES_WORDS + k] =
            frsqrte_bits((1023ULL + (k & 1)) << 52 | (r2 & 0xFFFFFFFFFFFFFULL));
    }
    for (e = 0; e < 2048; e++) {
        results[5 * SERIES_WORDS + 2 * e] = fres_bits(e << 52);
        results[5 * SERIES_WORDS + 2 * e + 1] = fres_bits(e << 52 | 0xFFFFFFFFFFFFFULL);
        results[6 * SERIES_WORDS + 2 * e] = frsqrte_bits(e << 52);
        results[6 * SERIES_WORDS + 2 * e + 1] = frsqrte_bits(e << 52 | 0xFFFFFFFFFFFFFULL);
    }

    for (i = 0; i < RN_WORDS; i++)
        rn_inputs[i] = 0x3FF0000000000000ULL | (lcg64() & 0xFFFFFFFFFFFFFULL);
    at = SERIES * SERIES_WORDS;
    for (rn = 0; rn < 4; rn++) {
        set_rn(rn);
        for (i = 0; i < RN_WORDS; i++)
            results[at++] = fres_bits(rn_inputs[i]);
        for (i = 0; i < RN_WORDS; i++)
            results[at++] = frsqrte_bits(rn_inputs[i]);
    }

    for (i = 0; i < SPECIALS; i++) {
        clear_fpscr();
        results[at++] = fres_bits(SPECIAL_INPUTS[i]);
        results[at++] = fpscr();
        clear_fpscr();
        results[at++] = frsqrte_bits(SPECIAL_INPUTS[i]);
        results[at++] = fpscr();
    }
    clear_fpscr();

    write_frame(results, sizeof(results));
    return 0;
}
