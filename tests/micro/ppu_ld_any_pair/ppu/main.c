/* PPU probe: the 64-bit registers that two adjacent "ld" loads leave when
 * they are not one base at d and d+8. Every destination starts at
 * 0xFFFFFFFFFFFFFFFF. One case reads two bases, one reads one base 24
 * bytes apart, one has the first load overwrite its own base while the
 * second load reads another base, one loads both into one register from a
 * falling address, one reads two bases with negative displacements, and
 * one has the first load write the second load's base.
 *
 * The probe sends no system call; word 0 stays 0.
 *
 * buf holds four doublewords: 0x0123456789ABCDEF, 0x8000000000000001,
 * 0xFEDCBA9876543210, 0x7FFFFFFFFFFFFFFE. alt holds 0x1111111122222222,
 * 0xA5A5A5A55A5A5A5A, 0x3333333344444444. ptrbuf holds the address of alt,
 * then 0x0F0E0D0C0B0A0908.
 *
 * Payload (big-endian u32 words; each register is high word then low):
 *   0  failed step (0 = none)   1  its return code
 *   2  r4 = buf, r7 = alt; ld 5,8(4) ; ld 6,8(7): r5 high   3  r5 low
 *   4  the same case: r6 high                               5  r6 low
 *   6  r4 = buf; ld 5,0(4) ; ld 6,24(4): r5 high            7  r5 low
 *   8  the same case: r6 high                               9  r6 low
 *  10  r4 = ptrbuf, r7 = buf; ld 4,8(4) ; ld 6,16(7): r4 high
 *                                                          11  r4 low
 *  12  the same case: r6 high                              13  r6 low
 *  14  r4 = buf; ld 5,16(4) ; ld 5,0(4): r5 high           15  r5 low
 *  16  r4 = buf + 24, r7 = alt + 24; ld 5,-16(4) ; ld 6,-8(7): r5 high
 *                                                          17  r5 low
 *  18  the same case: r6 high                              19  r6 low
 *  20  r4 = ptrbuf; ld 7,0(4) ; ld 6,16(7): r6 high        21  r6 low
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 22

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned long long buf[4] __attribute__((aligned(128)));
static volatile unsigned long long alt[3] __attribute__((aligned(128)));
static volatile unsigned long long ptrbuf[2] __attribute__((aligned(128)));

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
    unsigned long long a;
    unsigned long long b;

    memset((void *)result, 0, sizeof(result));
    buf[0] = 0x0123456789ABCDEFull;
    buf[1] = 0x8000000000000001ull;
    buf[2] = 0xFEDCBA9876543210ull;
    buf[3] = 0x7FFFFFFFFFFFFFFEull;
    alt[0] = 0x1111111122222222ull;
    alt[1] = 0xA5A5A5A55A5A5A5Aull;
    alt[2] = 0x3333333344444444ull;
    ptrbuf[0] = (unsigned long long)(unsigned long)alt;
    ptrbuf[1] = 0x0F0E0D0C0B0A0908ull;
    __sync_synchronize();

    __asm__ volatile ("mr 4,%2\n\t"
                      "mr 7,%3\n\t"
                      "li 5,-1\n\t"
                      "li 6,-1\n\t"
                      "ld 5,8(4)\n\t"
                      "ld 6,8(7)\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(a), "=&r"(b) : "b"(buf), "b"(alt)
                      : "r4", "r5", "r6", "r7", "memory");
    put(2, a);
    put(4, b);

    __asm__ volatile ("mr 4,%2\n\t"
                      "li 5,-1\n\t"
                      "li 6,-1\n\t"
                      "ld 5,0(4)\n\t"
                      "ld 6,24(4)\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(a), "=&r"(b) : "b"(buf) : "r4", "r5", "r6", "memory");
    put(6, a);
    put(8, b);

    __asm__ volatile ("mr 4,%2\n\t"
                      "mr 7,%3\n\t"
                      "li 6,-1\n\t"
                      "ld 4,8(4)\n\t"
                      "ld 6,16(7)\n\t"
                      "mr %0,4\n\t"
                      "mr %1,6"
                      : "=&r"(a), "=&r"(b) : "b"(ptrbuf), "b"(buf)
                      : "r4", "r6", "r7", "memory");
    put(10, a);
    put(12, b);

    __asm__ volatile ("mr 4,%1\n\t"
                      "li 5,-1\n\t"
                      "ld 5,16(4)\n\t"
                      "ld 5,0(4)\n\t"
                      "mr %0,5"
                      : "=&r"(a) : "b"(buf) : "r4", "r5", "memory");
    put(14, a);

    __asm__ volatile ("addi 4,%2,24\n\t"
                      "addi 7,%3,24\n\t"
                      "li 5,-1\n\t"
                      "li 6,-1\n\t"
                      "ld 5,-16(4)\n\t"
                      "ld 6,-8(7)\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(a), "=&r"(b) : "b"(buf), "b"(alt)
                      : "r4", "r5", "r6", "r7", "memory");
    put(16, a);
    put(18, b);

    __asm__ volatile ("mr 4,%1\n\t"
                      "li 6,-1\n\t"
                      "ld 7,0(4)\n\t"
                      "ld 6,16(7)\n\t"
                      "mr %0,6"
                      : "=&r"(a) : "b"(ptrbuf) : "r4", "r6", "r7", "memory");
    put(20, a);

    write_frame();
    return 0;
}
