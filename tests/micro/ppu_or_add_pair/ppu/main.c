/* PPU probe: the 64-bit registers that "or rA,rS,rB" followed by
 * "add rT,rA2,rB2" leaves, when the add reads the or's result as its
 * RA and as its RB, for the mr form of the or, and when the add's RT is
 * the or's RA. One more case branches over the or into the add, so the
 * add runs on the RA it finds.
 *
 * Every case loads its inputs into r6, r7 and r9 first, and r5 where it
 * names a start value.
 *
 * The probe sends no system call; word 0 stays 0.
 *
 * Payload (big-endian u32 words; each register is high word then low):
 *   0  failed step (0 = none)   1  its return code
 *   2  r6 = 0x8000000100000000, r7 = 0x00000000FFFFFFFF, r9 = 1,
 *      "or 5,6,7 ; add 8,5,9": r5 high      3  r5 low
 *   4  the same case: r8 high               5  r8 low
 *   6  r6 = all ones, r9 = 2, "mr 5,6 ; add 8,5,9": r5 high  7  r5 low
 *   8  the same case: r8 high               9  r8 low
 *  10  r5 = 0x0000000100000100, r6 = 0x10, r7 = 0x20, r9 = 3,
 *      "b 1f ; or 5,6,7 ; 1: add 8,5,9": r5 high   11  r5 low
 *  12  the same case: r8 high              13  r8 low
 *  14  r6 = 0x4000000000000000, r7 = 1, "or 5,6,7 ; add 5,5,5":
 *      r5 high                             15  r5 low
 *  16  r6 = 0x8000000100000000, r7 = 0x00000000FFFFFFFF, r9 = 1,
 *      "or 5,6,7 ; add 8,9,5": r8 high     17  r8 low
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 18

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));

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

int main(void)
{
    unsigned long long a, b;
    unsigned long long hi_bit = 0x8000000100000000ull;
    unsigned long long low_ones = 0x00000000FFFFFFFFull;
    unsigned long long all_ones = 0xFFFFFFFFFFFFFFFFull;
    unsigned long long start = 0x0000000100000100ull;
    unsigned long long top = 0x4000000000000000ull;

    memset((void *)result, 0, sizeof(result));

    __asm__ volatile ("mr 6,%2\n\t"
                      "mr 7,%3\n\t"
                      "li 9,1\n\t"
                      "or 5,6,7\n\t"
                      "add 8,5,9\n\t"
                      "mr %0,5\n\t"
                      "mr %1,8"
                      : "=&r"(a), "=&r"(b) : "r"(hi_bit), "r"(low_ones)
                      : "r5", "r6", "r7", "r8", "r9");
    put(2, a);
    put(4, b);

    __asm__ volatile ("mr 6,%2\n\t"
                      "li 9,2\n\t"
                      "mr 5,6\n\t"
                      "add 8,5,9\n\t"
                      "mr %0,5\n\t"
                      "mr %1,8"
                      : "=&r"(a), "=&r"(b) : "r"(all_ones)
                      : "r5", "r6", "r8", "r9");
    put(6, a);
    put(8, b);

    __asm__ volatile ("mr 5,%2\n\t"
                      "li 6,0x10\n\t"
                      "li 7,0x20\n\t"
                      "li 9,3\n\t"
                      "b 1f\n\t"
                      "or 5,6,7\n"
                      "1:\tadd 8,5,9\n\t"
                      "mr %0,5\n\t"
                      "mr %1,8"
                      : "=&r"(a), "=&r"(b) : "r"(start)
                      : "r5", "r6", "r7", "r8", "r9");
    put(10, a);
    put(12, b);

    __asm__ volatile ("mr 6,%1\n\t"
                      "li 7,1\n\t"
                      "or 5,6,7\n\t"
                      "add 5,5,5\n\t"
                      "mr %0,5"
                      : "=&r"(a) : "r"(top)
                      : "r5", "r6", "r7");
    put(14, a);

    __asm__ volatile ("mr 6,%1\n\t"
                      "mr 7,%2\n\t"
                      "li 9,1\n\t"
                      "or 5,6,7\n\t"
                      "add 8,9,5\n\t"
                      "mr %0,8"
                      : "=&r"(b) : "r"(hi_bit), "r"(low_ones)
                      : "r5", "r6", "r7", "r8", "r9");
    put(16, b);

    write_frame();
    return 0;
}
