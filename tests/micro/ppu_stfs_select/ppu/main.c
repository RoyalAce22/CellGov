/* PPU probe: the word stfs writes for doubles a single cannot hold:
 * above the single range, with fraction bits below single precision, in
 * the single denormal range, and below it.
 *
 * Each case loads FRS from memory with lfd and stores it with stfs.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code
 *   2 + c, the word stored for case c:
 *     0  1e300 (0x7E37E43C8800759C)
 *     1  -1e300
 *     2  the largest double (0x7FEFFFFFFFFFFFFF)
 *     3  1 + 2^-23 + 2^-24 (0x3FF0000030000000, the halfway point)
 *     4  1 + 2^-23 with FRS bits 35:63 all set (0x3FF000003FFFFFFF)
 *     5  biased exponent 896, all-ones fraction
 *     6  biased exponent 885, all-ones fraction
 *     7  biased exponent 874, all-ones fraction
 *     8  -(biased exponent 880, all-ones fraction)
 *     9  biased exponent 873, all-ones fraction (the book: undefined)
 *    10  the smallest positive double (0x0000000000000001)
 *    11  -(biased exponent 1, fraction 0)
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define CASES 12
#define WORDS (2 + CASES)

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned long long opnd __attribute__((aligned(8)));
static volatile unsigned int sout __attribute__((aligned(8)));

static const unsigned long long operands[CASES] = {
    0x7E37E43C8800759Cull,
    0xFE37E43C8800759Cull,
    0x7FEFFFFFFFFFFFFFull,
    0x3FF0000030000000ull,
    0x3FF000003FFFFFFFull,
    0x380FFFFFFFFFFFFFull,
    0x375FFFFFFFFFFFFFull,
    0x36AFFFFFFFFFFFFFull,
    0xB70FFFFFFFFFFFFFull,
    0x369FFFFFFFFFFFFFull,
    0x0000000000000001ull,
    0x8010000000000000ull,
};

static void write_frame(void)
{
    unsigned int written;
    unsigned int len = sizeof(result);
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)result, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)result, len);
}

int main(void)
{
    unsigned int c;

    memset((void *)result, 0, sizeof(result));
    for (c = 0; c < CASES; c++) {
        double f;
        opnd = operands[c];
        __asm__ volatile ("lfd %0,0(%1)\n\tstfs %0,0(%2)"
                          : "=&f"(f) : "b"(&opnd), "b"(&sout) : "memory");
        result[2 + c] = sout;
    }
    write_frame();
    return 0;
}
