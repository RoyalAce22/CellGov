/* PPU probe: the 64-bit registers that two adjacent "ld" loads from one
 * base, at d and d+8, leave. One case loads both into one register, one
 * branches over the first load into the second, one has the first load
 * overwrite the base so the second load reads through the new base, and
 * one uses negative displacements.
 *
 * The probe sends no system call; word 0 stays 0.
 *
 * Every case reads buf, except the base case, which reads ptrbuf and
 * then alt. buf holds four doublewords: 0x0123456789ABCDEF,
 * 0x8000000000000001, 0xFEDCBA9876543210, 0x7FFFFFFFFFFFFFFE. ptrbuf
 * holds the address of alt, then 0xDEADBEEFCAFEF00D. alt holds
 * 0x1111111122222222, then 0xA5A5A5A55A5A5A5A.
 *
 * Payload (big-endian u32 words; each register is high word then low):
 *   0  failed step (0 = none)   1  its return code
 *   2  r4 = buf; ld 5,0(4) ; ld 6,8(4): r5 high        3  r5 low
 *   4  the same case: r6 high                           5  r6 low
 *   6  r4 = buf; ld 5,8(4) ; ld 5,16(4): r5 high        7  r5 low
 *   8  r4 = buf, r5 = 0x0000000100000100, r6 = 0; "b 1f ; ld 5,0(4) ;
 *      1: ld 6,8(4)": r5 high                           9  r5 low
 *  10  the same case: r6 high                          11  r6 low
 *  12  r4 = ptrbuf; ld 4,0(4) ; ld 6,8(4): r6 high     13  r6 low
 *  14  r4 = buf + 24; ld 5,-16(4) ; ld 6,-8(4): r5 high
 *                                                      15  r5 low
 *  16  the same case: r6 high                          17  r6 low
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 18

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned long long buf[4] __attribute__((aligned(128)));
static volatile unsigned long long ptrbuf[2] __attribute__((aligned(128)));
static volatile unsigned long long alt[2] __attribute__((aligned(128)));

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
    ptrbuf[0] = (unsigned long long)(unsigned long)alt;
    ptrbuf[1] = 0xDEADBEEFCAFEF00Dull;
    __sync_synchronize();

    __asm__ volatile ("mr 4,%2\n\t"
                      "ld 5,0(4)\n\t"
                      "ld 6,8(4)\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(a), "=&r"(b) : "b"(buf) : "r4", "r5", "r6", "memory");
    put(2, a);
    put(4, b);

    __asm__ volatile ("mr 4,%1\n\t"
                      "ld 5,8(4)\n\t"
                      "ld 5,16(4)\n\t"
                      "mr %0,5"
                      : "=&r"(a) : "b"(buf) : "r4", "r5", "memory");
    put(6, a);

    __asm__ volatile ("mr 4,%2\n\t"
                      "li 5,1\n\t"
                      "sldi 5,5,32\n\t"
                      "ori 5,5,0x0100\n\t"
                      "li 6,0\n\t"
                      "b 1f\n\t"
                      "ld 5,0(4)\n"
                      "1:\tld 6,8(4)\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(a), "=&r"(b) : "b"(buf) : "r4", "r5", "r6", "memory");
    put(8, a);
    put(10, b);

    __asm__ volatile ("mr 4,%1\n\t"
                      "ld 4,0(4)\n\t"
                      "ld 6,8(4)\n\t"
                      "mr %0,6"
                      : "=&r"(a) : "b"(ptrbuf) : "r4", "r6", "memory");
    put(12, a);

    __asm__ volatile ("addi 4,%2,24\n\t"
                      "ld 5,-16(4)\n\t"
                      "ld 6,-8(4)\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(a), "=&r"(b) : "b"(buf) : "r4", "r5", "r6", "memory");
    put(14, a);
    put(16, b);

    write_frame();
    return 0;
}
