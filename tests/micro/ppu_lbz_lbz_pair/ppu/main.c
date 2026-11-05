/* PPU probe: the 64-bit registers that two adjacent "lbz" loads leave.
 * Every destination starts at 0xFFFFFFFFFFFFFFFF unless a case says
 * otherwise, so a register shows whether its upper 56 bits were cleared.
 * One case loads two bytes from one base, one loads both into one
 * register, one branches over the first load into the second, one has
 * the first load overwrite its own base while the second load reads
 * another base, one reads two bases with negative displacements, one has
 * the second load overwrite its own base, and one has the second load
 * overwrite the first load's base.
 *
 * The probe sends no system call; word 0 stays 0.
 *
 * buf holds eight bytes: 0x89 0x80 0x01 0x7F 0xFF 0x00 0x5A 0xA5.
 * alt holds four bytes: 0x11 0xC3 0x22 0xEE.
 *
 * Payload (big-endian u32 words; each register is high word then low):
 *   0  failed step (0 = none)   1  its return code
 *   2  r4 = buf; lbz 5,0(4) ; lbz 6,1(4): r5 high       3  r5 low
 *   4  the same case: r6 high                           5  r6 low
 *   6  r4 = buf; lbz 5,3(4) ; lbz 5,4(4): r5 high       7  r5 low
 *   8  r4 = buf, r5 = 0x0000000100000100, r6 = ~0; "b 1f ; lbz 5,0(4) ;
 *      1: lbz 6,1(4)": r5 high                          9  r5 low
 *  10  the same case: r6 high                          11  r6 low
 *  12  r4 = buf, r7 = alt; lbz 4,4(4) ; lbz 6,1(7): r4 high
 *                                                      13  r4 low
 *  14  the same case: r6 high                          15  r6 low
 *  16  r4 = buf + 8, r7 = alt + 4; lbz 5,-8(4) ; lbz 6,-3(7): r5 high
 *                                                      17  r5 low
 *  18  the same case: r6 high                          19  r6 low
 *  20  r4 = buf, r7 = alt; lbz 5,6(4) ; lbz 7,3(7): r5 high
 *                                                      21  r5 low
 *  22  the same case: r7 high                          23  r7 low
 *  24  r4 = buf, r7 = alt; lbz 5,7(4) ; lbz 4,2(7): r5 high
 *                                                      25  r5 low
 *  26  the same case: r4 high                          27  r4 low
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 28

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned char buf[8] __attribute__((aligned(128)));
static volatile unsigned char alt[4] __attribute__((aligned(128)));

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
    buf[0] = 0x89;
    buf[1] = 0x80;
    buf[2] = 0x01;
    buf[3] = 0x7F;
    buf[4] = 0xFF;
    buf[5] = 0x00;
    buf[6] = 0x5A;
    buf[7] = 0xA5;
    alt[0] = 0x11;
    alt[1] = 0xC3;
    alt[2] = 0x22;
    alt[3] = 0xEE;
    __sync_synchronize();

    __asm__ volatile ("mr 4,%2\n\t"
                      "li 5,-1\n\t"
                      "li 6,-1\n\t"
                      "lbz 5,0(4)\n\t"
                      "lbz 6,1(4)\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(a), "=&r"(b) : "b"(buf) : "r4", "r5", "r6", "memory");
    put(2, a);
    put(4, b);

    __asm__ volatile ("mr 4,%1\n\t"
                      "li 5,-1\n\t"
                      "lbz 5,3(4)\n\t"
                      "lbz 5,4(4)\n\t"
                      "mr %0,5"
                      : "=&r"(a) : "b"(buf) : "r4", "r5", "memory");
    put(6, a);

    __asm__ volatile ("mr 4,%2\n\t"
                      "li 5,1\n\t"
                      "sldi 5,5,32\n\t"
                      "ori 5,5,0x0100\n\t"
                      "li 6,-1\n\t"
                      "b 1f\n\t"
                      "lbz 5,0(4)\n"
                      "1:\tlbz 6,1(4)\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(a), "=&r"(b) : "b"(buf) : "r4", "r5", "r6", "memory");
    put(8, a);
    put(10, b);

    __asm__ volatile ("mr 4,%2\n\t"
                      "mr 7,%3\n\t"
                      "li 6,-1\n\t"
                      "lbz 4,4(4)\n\t"
                      "lbz 6,1(7)\n\t"
                      "mr %0,4\n\t"
                      "mr %1,6"
                      : "=&r"(a), "=&r"(b) : "b"(buf), "b"(alt)
                      : "r4", "r6", "r7", "memory");
    put(12, a);
    put(14, b);

    __asm__ volatile ("addi 4,%2,8\n\t"
                      "addi 7,%3,4\n\t"
                      "li 5,-1\n\t"
                      "li 6,-1\n\t"
                      "lbz 5,-8(4)\n\t"
                      "lbz 6,-3(7)\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(a), "=&r"(b) : "b"(buf), "b"(alt)
                      : "r4", "r5", "r6", "r7", "memory");
    put(16, a);
    put(18, b);

    __asm__ volatile ("mr 4,%2\n\t"
                      "mr 7,%3\n\t"
                      "li 5,-1\n\t"
                      "lbz 5,6(4)\n\t"
                      "lbz 7,3(7)\n\t"
                      "mr %0,5\n\t"
                      "mr %1,7"
                      : "=&r"(a), "=&r"(b) : "b"(buf), "b"(alt)
                      : "r4", "r5", "r7", "memory");
    put(20, a);
    put(22, b);

    __asm__ volatile ("mr 4,%2\n\t"
                      "mr 7,%3\n\t"
                      "li 5,-1\n\t"
                      "lbz 5,7(4)\n\t"
                      "lbz 4,2(7)\n\t"
                      "mr %0,5\n\t"
                      "mr %1,4"
                      : "=&r"(a), "=&r"(b) : "b"(buf), "b"(alt)
                      : "r4", "r5", "r7", "memory");
    put(24, a);
    put(26, b);

    write_frame();
    return 0;
}
