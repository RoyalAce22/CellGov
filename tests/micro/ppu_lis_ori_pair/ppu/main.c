/* PPU probe: the 64-bit register that "lis rT,hi" followed by
 * "ori rT,rT,lo" leaves, for a negative, a positive and an all-ones hi,
 * and for rT = r0 with lo = 0 and lo = 1. One more case branches over
 * the lis into the ori, so the ori runs on the register it finds.
 *
 * The probe sends no system call; word 0 stays 0.
 *
 * Payload (big-endian u32 words; each register is high word then low):
 *   0  failed step (0 = none)   1  its return code
 *   2  lis 5,0x8001 ; ori 5,5,0x00F0: r5 high   3  r5 low
 *   4  lis 5,0x7FFF ; ori 5,5,0xFFFF: r5 high   5  r5 low
 *   6  lis 5,0xFFFF ; ori 5,5,0xFFFF: r5 high   7  r5 low
 *   8  r5 = 0x0000000100000100, then "b 1f ; lis 5,0x1234 ;
 *      1: ori 5,5,0x00F0": r5 high              9  r5 low
 *  10  lis 0,0x8000 ; ori 0,0,0: r0 high       11  r0 low
 *  12  lis 0,0x8000 ; ori 0,0,1: r0 high       13  r0 low
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 14

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
    unsigned long long v;

    memset((void *)result, 0, sizeof(result));

    __asm__ volatile ("lis 5,0x8001\n\t"
                      "ori 5,5,0x00F0\n\t"
                      "mr %0,5"
                      : "=r"(v) : : "r5");
    put(2, v);

    __asm__ volatile ("lis 5,0x7FFF\n\t"
                      "ori 5,5,0xFFFF\n\t"
                      "mr %0,5"
                      : "=r"(v) : : "r5");
    put(4, v);

    __asm__ volatile ("lis 5,0xFFFF\n\t"
                      "ori 5,5,0xFFFF\n\t"
                      "mr %0,5"
                      : "=r"(v) : : "r5");
    put(6, v);

    __asm__ volatile ("li 5,1\n\t"
                      "sldi 5,5,32\n\t"
                      "ori 5,5,0x0100\n\t"
                      "b 1f\n\t"
                      "lis 5,0x1234\n"
                      "1:\tori 5,5,0x00F0\n\t"
                      "mr %0,5"
                      : "=r"(v) : : "r5");
    put(8, v);

    __asm__ volatile ("lis 0,0x8000\n\t"
                      "ori 0,0,0\n\t"
                      "mr %0,0"
                      : "=r"(v) : : "r0");
    put(10, v);

    __asm__ volatile ("lis 0,0x8000\n\t"
                      "ori 0,0,1\n\t"
                      "mr %0,0"
                      : "=r"(v) : : "r0");
    put(12, v);

    write_frame();
    return 0;
}
