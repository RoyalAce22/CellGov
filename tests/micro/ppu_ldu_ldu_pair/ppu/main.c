/* PPU probe: what two adjacent "ldu rT,D(rA)" leave -- each loaded
 * doubleword and each updated base -- when both walk one base, when they
 * walk two bases, when the second target is the first base, when both
 * target one register, when a branch over the first runs the second
 * alone, and when one base walks back with negative offsets.
 *
 * A base is recorded as its distance from the value the case started it
 * at, so the payload names no console address.
 *
 * The probe sends no system call; word 0 stays 0.
 *
 * data[] holds D0 D1 D2 D3; d = &data[0].
 *
 * Payload (big-endian u32 words; each register is high word then low):
 *   0  failed step (0 = none)   1  its return code
 *   one base: r5 = d - 8, "ldu 7,8(5) ; ldu 8,8(5)":
 *      2-3 r7   4-5 r8   6-7 r5 - (d - 8)
 *   two bases: r5 = d - 8, r6 = d + 8, "ldu 7,8(5) ; ldu 8,8(6)":
 *      8-9 r7   10-11 r8   12-13 r5 - (d - 8)   14-15 r6 - (d + 8)
 *   second target is the first base: r5 = d - 8, r6 = d + 8,
 *   "ldu 7,8(5) ; ldu 5,8(6)":
 *     16-17 r7  18-19 r5   20-21 r6 - (d + 8)
 *   one target: r5 = d - 8, r6 = d + 8, "ldu 7,8(5) ; ldu 7,8(6)":
 *     22-23 r7  24-25 r5 - (d - 8)   26-27 r6 - (d + 8)
 *   a branch over the first: r7 = 0x77, r5 = d - 8, r6 = d + 8,
 *   "b 1f ; ldu 7,8(5) ; 1: ldu 8,8(6)":
 *     28-29 r7  30-31 r8   32-33 r5 - (d - 8)   34-35 r6 - (d + 8)
 *   negative offsets: r5 = d + 32, "ldu 7,-8(5) ; ldu 8,-8(5)":
 *     36-37 r7  38-39 r8   40-41 r5 - (d + 32)
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 42

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));

static volatile unsigned long long data[4] __attribute__((aligned(16))) = {
    0x0123456789ABCDEFull,
    0xFEDCBA9876543210ull,
    0x8000000000000001ull,
    0x7FFFFFFFFFFFFFFEull,
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

static void put(unsigned int word, unsigned long long v)
{
    result[word] = (unsigned int)(v >> 32);
    result[word + 1] = (unsigned int)v;
}

int main(void)
{
    unsigned long long r5, r6, r7, r8;
    unsigned long long d = (unsigned long long)(unsigned long)data;
    unsigned long long a0 = d - 8;
    unsigned long long b0 = d + 8;
    unsigned long long n0 = d + 32;

    memset((void *)result, 0, sizeof(result));

    __asm__ volatile ("mr 5,%3\n\t"
                      "ldu 7,8(5)\n\t"
                      "ldu 8,8(5)\n\t"
                      "mr %0,7\n\t"
                      "mr %1,8\n\t"
                      "mr %2,5"
                      : "=&r"(r7), "=&r"(r8), "=&r"(r5)
                      : "r"(a0)
                      : "r5", "r7", "r8", "memory");
    put(2, r7);
    put(4, r8);
    put(6, r5 - a0);

    __asm__ volatile ("mr 5,%4\n\t"
                      "mr 6,%5\n\t"
                      "ldu 7,8(5)\n\t"
                      "ldu 8,8(6)\n\t"
                      "mr %0,7\n\t"
                      "mr %1,8\n\t"
                      "mr %2,5\n\t"
                      "mr %3,6"
                      : "=&r"(r7), "=&r"(r8), "=&r"(r5), "=&r"(r6)
                      : "r"(a0), "r"(b0)
                      : "r5", "r6", "r7", "r8", "memory");
    put(8, r7);
    put(10, r8);
    put(12, r5 - a0);
    put(14, r6 - b0);

    __asm__ volatile ("mr 5,%3\n\t"
                      "mr 6,%4\n\t"
                      "ldu 7,8(5)\n\t"
                      "ldu 5,8(6)\n\t"
                      "mr %0,7\n\t"
                      "mr %1,5\n\t"
                      "mr %2,6"
                      : "=&r"(r7), "=&r"(r5), "=&r"(r6)
                      : "r"(a0), "r"(b0)
                      : "r5", "r6", "r7", "memory");
    put(16, r7);
    put(18, r5);
    put(20, r6 - b0);

    __asm__ volatile ("mr 5,%3\n\t"
                      "mr 6,%4\n\t"
                      "ldu 7,8(5)\n\t"
                      "ldu 7,8(6)\n\t"
                      "mr %0,7\n\t"
                      "mr %1,5\n\t"
                      "mr %2,6"
                      : "=&r"(r7), "=&r"(r5), "=&r"(r6)
                      : "r"(a0), "r"(b0)
                      : "r5", "r6", "r7", "memory");
    put(22, r7);
    put(24, r5 - a0);
    put(26, r6 - b0);

    __asm__ volatile ("mr 5,%4\n\t"
                      "mr 6,%5\n\t"
                      "li 7,0x77\n\t"
                      "b 1f\n\t"
                      "ldu 7,8(5)\n"
                      "1:\tldu 8,8(6)\n\t"
                      "mr %0,7\n\t"
                      "mr %1,8\n\t"
                      "mr %2,5\n\t"
                      "mr %3,6"
                      : "=&r"(r7), "=&r"(r8), "=&r"(r5), "=&r"(r6)
                      : "r"(a0), "r"(b0)
                      : "r5", "r6", "r7", "r8", "memory");
    put(28, r7);
    put(30, r8);
    put(32, r5 - a0);
    put(34, r6 - b0);

    __asm__ volatile ("mr 5,%3\n\t"
                      "ldu 7,-8(5)\n\t"
                      "ldu 8,-8(5)\n\t"
                      "mr %0,7\n\t"
                      "mr %1,8\n\t"
                      "mr %2,5"
                      : "=&r"(r7), "=&r"(r8), "=&r"(r5)
                      : "r"(n0)
                      : "r5", "r7", "r8", "memory");
    put(36, r7);
    put(38, r8);
    put(40, r5 - n0);

    write_frame();
    return 0;
}
