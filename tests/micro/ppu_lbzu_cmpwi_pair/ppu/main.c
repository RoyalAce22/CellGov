/* PPU probe: what "lbzu rT,D(rA)" followed by "cmpwi crF,rC,0" leaves --
 * the loaded byte, the updated base and the compare field -- when the
 * compare reads the loaded byte, the updated base and another register,
 * with XER[SO] set, and when a branch over the lbzu runs the compare
 * alone.
 *
 * A base is recorded as its distance from the value the case started it
 * at, so the payload names no console address. A CR field is recorded as
 * its LT GT EQ SO nibble.
 *
 * The probe sends no system call; word 0 stays 0.
 *
 * Payload (big-endian u32 words; each register is high word then low):
 *   0  failed step (0 = none)   1  its return code
 *   2  r5 = str - 1, r4 = all ones, "1: lbzu 4,1(5) ; cmpwi 7,4,0 ;
 *      bne 7,1b" over "abc\0": r4 high       3  r4 low
 *   4  the same case: r5 - (str - 1) high   5  low
 *   6  the same case: cr7
 *   7  r5 = hi - 1 (*hi = 0x80), r4 = all ones, XER[SO] = 1,
 *      "lbzu 4,1(5) ; cmpwi 0,4,0": r4 high  8  r4 low
 *   9  the same case: r5 - (hi - 1) high    10  low
 *  11  the same case: cr0
 *  12  r5 = hi - 1, "lbzu 4,1(5) ; cmpwi 7,5,0": cr7
 *  13  the same case: bit 31 of r5's low word (the sign cmpwi reads)
 *  14  r5 = hi - 1, r6 = 0xFFFFFFFF00000000, "lbzu 4,1(5) ; cmpwi 1,6,0":
 *      cr1
 *  15  r5 = hi - 1, r6 = 0x0000000080000000, "lbzu 4,1(5) ; cmpwi 7,6,0":
 *      cr7
 *  16  r5 = hi - 1, r4 = 0x44, "b 1f ; lbzu 4,1(5) ; 1: cmpwi 7,4,0":
 *      r4 high                              17  r4 low
 *  18  the same case: r5 - (hi - 1) high    19  low
 *  20  the same case: cr7
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 21

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));

static volatile unsigned char str[4] __attribute__((aligned(16))) =
    { 'a', 'b', 'c', 0 };
static volatile unsigned char hi[4] __attribute__((aligned(16))) =
    { 0x80, 0, 0, 0 };

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

static void put(unsigned int word, unsigned long long v)
{
    result[word] = (unsigned int)(v >> 32);
    result[word + 1] = (unsigned int)v;
}

/* CR field `bf` of `cr` as its LT GT EQ SO nibble. */
static unsigned int field(unsigned int cr, unsigned int bf)
{
    return (cr >> (4 * (7 - bf))) & 0xF;
}

int main(void)
{
    unsigned long long r4, r5;
    unsigned int cr;
    unsigned long long s0 = (unsigned long long)(unsigned long)str - 1;
    unsigned long long h0 = (unsigned long long)(unsigned long)hi - 1;
    unsigned long long all_ones = 0xFFFFFFFFFFFFFFFFull;
    unsigned long long low_zero = 0xFFFFFFFF00000000ull;
    unsigned long long low_neg = 0x0000000080000000ull;

    memset((void *)result, 0, sizeof(result));

    __asm__ volatile ("mr 5,%3\n\t"
                      "mr 4,%4\n"
                      "1:\tlbzu 4,1(5)\n\t"
                      "cmpwi 7,4,0\n\t"
                      "bne 7,1b\n\t"
                      "mfcr %2\n\t"
                      "mr %0,4\n\t"
                      "mr %1,5"
                      : "=&r"(r4), "=&r"(r5), "=&r"(cr)
                      : "r"(s0), "r"(all_ones)
                      : "r4", "r5", "cr7", "memory");
    put(2, r4);
    put(4, r5 - s0);
    result[6] = field(cr, 7);

    __asm__ volatile ("mr 5,%3\n\t"
                      "mr 4,%4\n\t"
                      "lis 6,-32768\n\t"
                      "mtxer 6\n\t"
                      "lbzu 4,1(5)\n\t"
                      "cmpwi 0,4,0\n\t"
                      "mfcr %2\n\t"
                      "li 6,0\n\t"
                      "mtxer 6\n\t"
                      "mr %0,4\n\t"
                      "mr %1,5"
                      : "=&r"(r4), "=&r"(r5), "=&r"(cr)
                      : "r"(h0), "r"(all_ones)
                      : "r4", "r5", "r6", "cr0", "memory");
    put(7, r4);
    put(9, r5 - h0);
    result[11] = field(cr, 0);

    __asm__ volatile ("mr 5,%2\n\t"
                      "lbzu 4,1(5)\n\t"
                      "cmpwi 7,5,0\n\t"
                      "mfcr %1\n\t"
                      "mr %0,5"
                      : "=&r"(r5), "=&r"(cr)
                      : "r"(h0)
                      : "r4", "r5", "cr7", "memory");
    result[12] = field(cr, 7);
    result[13] = ((unsigned int)r5) >> 31;

    __asm__ volatile ("mr 5,%1\n\t"
                      "mr 6,%2\n\t"
                      "lbzu 4,1(5)\n\t"
                      "cmpwi 1,6,0\n\t"
                      "mfcr %0"
                      : "=&r"(cr)
                      : "r"(h0), "r"(low_zero)
                      : "r4", "r5", "r6", "cr1", "memory");
    result[14] = field(cr, 1);

    __asm__ volatile ("mr 5,%1\n\t"
                      "mr 6,%2\n\t"
                      "lbzu 4,1(5)\n\t"
                      "cmpwi 7,6,0\n\t"
                      "mfcr %0"
                      : "=&r"(cr)
                      : "r"(h0), "r"(low_neg)
                      : "r4", "r5", "r6", "cr7", "memory");
    result[15] = field(cr, 7);

    __asm__ volatile ("mr 5,%3\n\t"
                      "li 4,0x44\n\t"
                      "b 1f\n\t"
                      "lbzu 4,1(5)\n"
                      "1:\tcmpwi 7,4,0\n\t"
                      "mfcr %2\n\t"
                      "mr %0,4\n\t"
                      "mr %1,5"
                      : "=&r"(r4), "=&r"(r5), "=&r"(cr)
                      : "r"(h0)
                      : "r4", "r5", "cr7", "memory");
    put(16, r4);
    put(18, r5 - h0);
    result[20] = field(cr, 7);

    write_frame();
    return 0;
}
