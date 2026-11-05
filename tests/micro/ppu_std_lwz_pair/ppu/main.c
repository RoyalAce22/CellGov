/* PPU probe: what an adjacent "std rS,d(r1) ; lwz rT,d2(rA)" leaves in
 * the registers and in memory. Each case points r1 at buf for the pair
 * and restores it in the same asm block; nothing between the two moves
 * touches the stack. r5 holds 0x89ABCDEF80000001 and every destination
 * starts at 0xFFFFFFFFFFFFFFFF, so a register shows whether its high word
 * was cleared.
 *
 * One case loads the high word the std stored, one the low word, one
 * loads into the stored register, one loads from another base, one uses
 * negative displacements, and one branches over the std into the lwz.
 *
 * The probe sends no system call; word 0 stays 0.
 *
 * Before each case buf holds 0xA1A2A3A4, 0xB1B2B3B4, 0xC1C2C3C4,
 * 0xD1D2D3D4. alt holds 0x11112222, then 0xA5A55A5A.
 *
 * Payload (big-endian u32 words; each register is high word then low):
 *   0  failed step (0 = none)   1  its return code
 *   2  r1 = buf; std 5,8(1) ; lwz 6,8(1): r6 high        3  r6 low
 *   4  r1 = buf; std 5,8(1) ; lwz 6,12(1): r6 high       5  r6 low
 *   6  r1 = buf; std 5,8(1) ; lwz 5,8(1): r5 high        7  r5 low
 *   8  r1 = buf, r4 = alt; std 5,8(1) ; lwz 6,4(4): r6 high
 *                                                        9  r6 low
 *  10  the same case: buf word 2 after it               11  buf word 3
 *  12  r1 = buf + 16; std 5,-8(1) ; lwz 6,-4(1): r6 high
 *                                                       13  r6 low
 *  14  r1 = buf; "b 1f ; std 5,8(1) ; 1: lwz 6,8(1)": r6 high
 *                                                       15  r6 low
 *  16  the same case: buf word 2 after it               17  buf word 3
 *  18  the first case: buf word 2 after it              19  buf word 3
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 20

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned int buf[4] __attribute__((aligned(128)));
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

static void reset_buf(void)
{
    buf[0] = 0xA1A2A3A4u;
    buf[1] = 0xB1B2B3B4u;
    buf[2] = 0xC1C2C3C4u;
    buf[3] = 0xD1D2D3D4u;
    __sync_synchronize();
}

int main(void)
{
    const unsigned long long v = 0x89ABCDEF80000001ull;
    unsigned long long a;

    memset((void *)result, 0, sizeof(result));
    alt[0] = 0x11112222u;
    alt[1] = 0xA5A55A5Au;

    reset_buf();
    __asm__ volatile ("li 6,-1\n\t"
                      "mr 5,%2\n\t"
                      "mr 9,1\n\t"
                      "mr 1,%1\n\t"
                      "std 5,8(1)\n\t"
                      "lwz 6,8(1)\n\t"
                      "mr 1,9\n\t"
                      "mr %0,6"
                      : "=&r"(a) : "b"(buf), "r"(v) : "r5", "r6", "r9", "memory");
    put(2, a);
    result[18] = buf[2];
    result[19] = buf[3];

    reset_buf();
    __asm__ volatile ("li 6,-1\n\t"
                      "mr 5,%2\n\t"
                      "mr 9,1\n\t"
                      "mr 1,%1\n\t"
                      "std 5,8(1)\n\t"
                      "lwz 6,12(1)\n\t"
                      "mr 1,9\n\t"
                      "mr %0,6"
                      : "=&r"(a) : "b"(buf), "r"(v) : "r5", "r6", "r9", "memory");
    put(4, a);

    reset_buf();
    __asm__ volatile ("mr 5,%2\n\t"
                      "mr 9,1\n\t"
                      "mr 1,%1\n\t"
                      "std 5,8(1)\n\t"
                      "lwz 5,8(1)\n\t"
                      "mr 1,9\n\t"
                      "mr %0,5"
                      : "=&r"(a) : "b"(buf), "r"(v) : "r5", "r9", "memory");
    put(6, a);

    reset_buf();
    __asm__ volatile ("li 6,-1\n\t"
                      "mr 5,%2\n\t"
                      "mr 4,%3\n\t"
                      "mr 9,1\n\t"
                      "mr 1,%1\n\t"
                      "std 5,8(1)\n\t"
                      "lwz 6,4(4)\n\t"
                      "mr 1,9\n\t"
                      "mr %0,6"
                      : "=&r"(a) : "b"(buf), "r"(v), "b"(alt)
                      : "r4", "r5", "r6", "r9", "memory");
    put(8, a);
    result[10] = buf[2];
    result[11] = buf[3];

    reset_buf();
    __asm__ volatile ("li 6,-1\n\t"
                      "mr 5,%2\n\t"
                      "mr 9,1\n\t"
                      "addi 1,%1,16\n\t"
                      "std 5,-8(1)\n\t"
                      "lwz 6,-4(1)\n\t"
                      "mr 1,9\n\t"
                      "mr %0,6"
                      : "=&r"(a) : "b"(buf), "r"(v) : "r5", "r6", "r9", "memory");
    put(12, a);

    reset_buf();
    __asm__ volatile ("li 6,-1\n\t"
                      "mr 5,%2\n\t"
                      "mr 9,1\n\t"
                      "mr 1,%1\n\t"
                      "b 1f\n\t"
                      "std 5,8(1)\n"
                      "1:\tlwz 6,8(1)\n\t"
                      "mr 1,9\n\t"
                      "mr %0,6"
                      : "=&r"(a) : "b"(buf), "r"(v) : "r5", "r6", "r9", "memory");
    put(14, a);
    result[16] = buf[2];
    result[17] = buf[3];

    write_frame();
    return 0;
}
