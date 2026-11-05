/* PPU probe: the 64-bit registers that two adjacent "lwz" loads leave.
 * Every destination starts at 0xFFFFFFFFFFFFFFFF, so a register shows
 * whether its high word was cleared. One case loads two words from one
 * base, one loads both into one register, one branches over the first
 * load into the second, one has the first load overwrite the base the
 * second load reads, one has the first load overwrite its own base while
 * the second load reads another base, and one reads two bases with
 * negative displacements.
 *
 * The probe sends no system call; word 0 stays 0.
 *
 * buf holds six words: 0x89ABCDEF, 0x80000001, 0x76543210, 0xFFFFFFFE,
 * 0x0000FFFF, 0x12345678. ptrbuf holds the address of alt, then
 * 0xDEADBEEF. alt holds 0x11112222, then 0xA5A55A5A.
 *
 * Payload (big-endian u32 words; each register is high word then low):
 *   0  failed step (0 = none)   1  its return code
 *   2  r4 = buf; lwz 5,0(4) ; lwz 6,4(4): r5 high       3  r5 low
 *   4  the same case: r6 high                           5  r6 low
 *   6  r4 = buf; lwz 5,4(4) ; lwz 5,8(4): r5 high       7  r5 low
 *   8  r4 = buf, r5 = 0x0000000100000100, r6 = ~0; "b 1f ; lwz 5,0(4) ;
 *      1: lwz 6,4(4)": r5 high                          9  r5 low
 *  10  the same case: r6 high                          11  r6 low
 *  12  r4 = ptrbuf; lwz 4,0(4) ; lwz 6,4(4): r6 high   13  r6 low
 *  14  r4 = ptrbuf, r7 = buf; lwz 4,4(4) ; lwz 6,12(7): r4 high
 *                                                      15  r4 low
 *  16  the same case: r6 high                          17  r6 low
 *  18  r4 = buf + 24, r7 = alt + 8; lwz 5,-4(4) ; lwz 6,-4(7): r5 high
 *                                                      19  r5 low
 *  20  the same case: r6 high                          21  r6 low
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 22

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned int buf[6] __attribute__((aligned(128)));
static volatile unsigned int ptrbuf[2] __attribute__((aligned(128)));
static volatile unsigned int alt[2] __attribute__((aligned(128)));

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
    buf[0] = 0x89ABCDEFu;
    buf[1] = 0x80000001u;
    buf[2] = 0x76543210u;
    buf[3] = 0xFFFFFFFEu;
    buf[4] = 0x0000FFFFu;
    buf[5] = 0x12345678u;
    alt[0] = 0x11112222u;
    alt[1] = 0xA5A55A5Au;
    ptrbuf[0] = (unsigned int)(unsigned long)alt;
    ptrbuf[1] = 0xDEADBEEFu;
    __sync_synchronize();

    __asm__ volatile ("mr 4,%2\n\t"
                      "li 5,-1\n\t"
                      "li 6,-1\n\t"
                      "lwz 5,0(4)\n\t"
                      "lwz 6,4(4)\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(a), "=&r"(b) : "b"(buf) : "r4", "r5", "r6", "memory");
    put(2, a);
    put(4, b);

    __asm__ volatile ("mr 4,%1\n\t"
                      "li 5,-1\n\t"
                      "lwz 5,4(4)\n\t"
                      "lwz 5,8(4)\n\t"
                      "mr %0,5"
                      : "=&r"(a) : "b"(buf) : "r4", "r5", "memory");
    put(6, a);

    __asm__ volatile ("mr 4,%2\n\t"
                      "li 5,1\n\t"
                      "sldi 5,5,32\n\t"
                      "ori 5,5,0x0100\n\t"
                      "li 6,-1\n\t"
                      "b 1f\n\t"
                      "lwz 5,0(4)\n"
                      "1:\tlwz 6,4(4)\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(a), "=&r"(b) : "b"(buf) : "r4", "r5", "r6", "memory");
    put(8, a);
    put(10, b);

    __asm__ volatile ("mr 4,%1\n\t"
                      "li 6,-1\n\t"
                      "lwz 4,0(4)\n\t"
                      "lwz 6,4(4)\n\t"
                      "mr %0,6"
                      : "=&r"(a) : "b"(ptrbuf) : "r4", "r6", "memory");
    put(12, a);

    __asm__ volatile ("mr 4,%2\n\t"
                      "mr 7,%3\n\t"
                      "li 6,-1\n\t"
                      "lwz 4,4(4)\n\t"
                      "lwz 6,12(7)\n\t"
                      "mr %0,4\n\t"
                      "mr %1,6"
                      : "=&r"(a), "=&r"(b) : "b"(ptrbuf), "b"(buf)
                      : "r4", "r6", "r7", "memory");
    put(14, a);
    put(16, b);

    __asm__ volatile ("addi 4,%2,24\n\t"
                      "addi 7,%3,8\n\t"
                      "li 5,-1\n\t"
                      "li 6,-1\n\t"
                      "lwz 5,-4(4)\n\t"
                      "lwz 6,-4(7)\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(a), "=&r"(b) : "b"(buf), "b"(alt)
                      : "r4", "r5", "r6", "r7", "memory");
    put(18, a);
    put(20, b);

    write_frame();
    return 0;
}
