/* PPU probe: what an adjacent "cmpd crF,rA,rB ; nor rA2,rS,rB2" leaves --
 * the whole CR and the nor's target -- with a compare that is greater
 * only in 64 bits, a less-than on cr0 with a two-source nor, an equal
 * compare whose nor overwrites the compare's first source, a compare
 * with XER[SO] set, and a branch over the cmpd that runs the nor alone.
 *
 * Each case starts with every CR field 0 and XER 0, except the SO case,
 * which starts with XER = 0x80000000 and puts XER back to 0 after.
 *
 * The probe sends no system call; word 0 stays 0.
 *
 * Payload (big-endian u32 words; each register is high word then low):
 *   0  failed step (0 = none)   1  its return code
 *   64-bit greater: r3 = 0x1_0000_0000, r4 = 1, r6 = 0xF0,
 *   "cmpd 7,3,4 ; nor 5,6,6":
 *      2 CR   3-4 r5
 *   less on cr0: r3 = -1, r4 = 1, r6 = 0xF0F0F0F0F0F0F0F0,
 *   r8 = 0x0F000000000000FF, "cmpd 0,3,4 ; nor 5,6,8":
 *      5 CR   6-7 r5
 *   equal, nor target is the first source: r3 = r4 = 0x8000000000000000,
 *   r6 = 0x00FF, "cmpd 7,3,4 ; nor 3,6,6":
 *      8 CR   9-10 r3
 *   SO set: XER = 0x80000000, r3 = 1, r4 = 2, r6 = 0xF0,
 *   "cmpd 7,3,4 ; nor 5,6,6":
 *     11 CR  12-13 r5
 *   a branch over the cmpd: r3 = r4 = 1, r6 = 0xF0,
 *   "b 1f ; cmpd 7,3,4 ; 1: nor 5,6,6":
 *     14 CR  15-16 r5
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 17

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

#define CR_CLOBBERS "cr0", "cr1", "cr2", "cr3", "cr4", "cr5", "cr6", "cr7"

int main(void)
{
    unsigned long long cr, r3, r5;
    unsigned long long zero = 0;

    memset((void *)result, 0, sizeof(result));

    __asm__ volatile ("mtcrf 255,%2\n\t"
                      "mtxer %2\n\t"
                      "mr 3,%3\n\t"
                      "mr 4,%4\n\t"
                      "mr 6,%5\n\t"
                      "cmpd 7,3,4\n\t"
                      "nor 5,6,6\n\t"
                      "mfcr %0\n\t"
                      "mr %1,5"
                      : "=&r"(cr), "=&r"(r5)
                      : "r"(zero), "r"(0x100000000ull), "r"(1ull),
                        "r"(0xF0ull)
                      : "r3", "r4", "r5", "r6", "xer", CR_CLOBBERS);
    result[2] = (unsigned int)cr;
    put(3, r5);

    __asm__ volatile ("mtcrf 255,%2\n\t"
                      "mtxer %2\n\t"
                      "mr 3,%3\n\t"
                      "mr 4,%4\n\t"
                      "mr 6,%5\n\t"
                      "mr 8,%6\n\t"
                      "cmpd 0,3,4\n\t"
                      "nor 5,6,8\n\t"
                      "mfcr %0\n\t"
                      "mr %1,5"
                      : "=&r"(cr), "=&r"(r5)
                      : "r"(zero), "r"(~0ull), "r"(1ull),
                        "r"(0xF0F0F0F0F0F0F0F0ull), "r"(0x0F000000000000FFull)
                      : "r3", "r4", "r5", "r6", "r8", "xer", CR_CLOBBERS);
    result[5] = (unsigned int)cr;
    put(6, r5);

    __asm__ volatile ("mtcrf 255,%2\n\t"
                      "mtxer %2\n\t"
                      "mr 3,%3\n\t"
                      "mr 4,%3\n\t"
                      "mr 6,%4\n\t"
                      "cmpd 7,3,4\n\t"
                      "nor 3,6,6\n\t"
                      "mfcr %0\n\t"
                      "mr %1,3"
                      : "=&r"(cr), "=&r"(r3)
                      : "r"(zero), "r"(0x8000000000000000ull), "r"(0xFFull)
                      : "r3", "r4", "r6", "xer", CR_CLOBBERS);
    result[8] = (unsigned int)cr;
    put(9, r3);

    __asm__ volatile ("mtcrf 255,%2\n\t"
                      "mtxer %3\n\t"
                      "mr 3,%4\n\t"
                      "mr 4,%5\n\t"
                      "mr 6,%6\n\t"
                      "cmpd 7,3,4\n\t"
                      "nor 5,6,6\n\t"
                      "mfcr %0\n\t"
                      "mr %1,5\n\t"
                      "mtxer %2"
                      : "=&r"(cr), "=&r"(r5)
                      : "r"(zero), "r"(0x80000000ull), "r"(1ull), "r"(2ull),
                        "r"(0xF0ull)
                      : "r3", "r4", "r5", "r6", "xer", CR_CLOBBERS);
    result[11] = (unsigned int)cr;
    put(12, r5);

    __asm__ volatile ("mtcrf 255,%2\n\t"
                      "mtxer %2\n\t"
                      "mr 3,%3\n\t"
                      "mr 4,%3\n\t"
                      "mr 6,%4\n\t"
                      "b 1f\n\t"
                      "cmpd 7,3,4\n"
                      "1:\tnor 5,6,6\n\t"
                      "mfcr %0\n\t"
                      "mr %1,5"
                      : "=&r"(cr), "=&r"(r5)
                      : "r"(zero), "r"(1ull), "r"(0xF0ull)
                      : "r3", "r4", "r5", "r6", "xer", CR_CLOBBERS);
    result[14] = (unsigned int)cr;
    put(15, r5);

    write_frame();
    return 0;
}
