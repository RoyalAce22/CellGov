/* PPU probe: what an adjacent "addi rT,rA,SI ; bc" leaves -- the addi's
 * target, the CTR and a register the branch skips or runs -- for a counted
 * bdnz loop, an li (addi from RA = 0) with r0 nonzero, an addi that wraps
 * below 0 followed by a bc not taken, a bdnz to its own address, and a
 * branch into the bdnz that runs it alone.
 *
 * Each case starts with every CR field 0.
 *
 * The probe sends no system call; word 0 stays 0.
 *
 * Payload (big-endian u32 words; each register is high word then low):
 *   0  failed step (0 = none)   1  its return code
 *   counted loop: r5 = 0, CTR = 3, "0: addi 5,5,1 ; bdnz 0b":
 *      2-3 r5   4-5 CTR
 *   li with r0 = 0x100, r7 = 0, "li 5,-3 ; bne 0,1f ; li 7,1 ; 1:":
 *      6-7 r5   8-9 r7
 *   a wrap and a beq that falls through: r6 = 1, r7 = 0, "addi 5,6,-3 ; beq 0,1f ; li 7,1 ; 1:":
 *     10-11 r5  12-13 r7
 *   bdnz to its own address: r5 = 0, CTR = 3, "addi 5,5,1 ; 0: bdnz 0b":
 *     14-15 r5  16-17 CTR
 *   a branch into the bdnz: r5 = r6 = 0, CTR = 3,
 *   "0: b 1f ; addi 5,5,1 ; 1: bdnz 0b ; addi 6,6,1":
 *     18-19 r5  20-21 r6  22-23 CTR
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 24

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
    unsigned long long r5, r6, r7, ctr;
    unsigned long long zero = 0;

    memset((void *)result, 0, sizeof(result));

    __asm__ volatile ("mtcrf 255,%2\n\t"
                      "li 5,0\n\t"
                      "mtctr %3\n"
                      "0:\taddi 5,5,1\n\t"
                      "bdnz 0b\n\t"
                      "mr %0,5\n\t"
                      "mfctr %1"
                      : "=&r"(r5), "=&r"(ctr)
                      : "r"(zero), "r"(3ull)
                      : "r5", "ctr", CR_CLOBBERS);
    put(2, r5);
    put(4, ctr);

    __asm__ volatile ("mtcrf 255,%2\n\t"
                      "mr 0,%3\n\t"
                      "li 7,0\n\t"
                      "addi 5,0,-3\n\t"
                      "bc 4,2,1f\n\t"
                      "li 7,1\n"
                      "1:\tmr %0,5\n\t"
                      "mr %1,7"
                      : "=&r"(r5), "=&r"(r7)
                      : "r"(zero), "r"(0x100ull)
                      : "r0", "r5", "r7", CR_CLOBBERS);
    put(6, r5);
    put(8, r7);

    __asm__ volatile ("mtcrf 255,%2\n\t"
                      "mr 6,%3\n\t"
                      "li 7,0\n\t"
                      "addi 5,6,-3\n\t"
                      "bc 12,2,1f\n\t"
                      "li 7,1\n"
                      "1:\tmr %0,5\n\t"
                      "mr %1,7"
                      : "=&r"(r5), "=&r"(r7)
                      : "r"(zero), "r"(1ull)
                      : "r5", "r6", "r7", CR_CLOBBERS);
    put(10, r5);
    put(12, r7);

    __asm__ volatile ("mtcrf 255,%2\n\t"
                      "li 5,0\n\t"
                      "mtctr %3\n\t"
                      "addi 5,5,1\n"
                      "0:\tbdnz 0b\n\t"
                      "mr %0,5\n\t"
                      "mfctr %1"
                      : "=&r"(r5), "=&r"(ctr)
                      : "r"(zero), "r"(3ull)
                      : "r5", "ctr", CR_CLOBBERS);
    put(14, r5);
    put(16, ctr);

    __asm__ volatile ("mtcrf 255,%3\n\t"
                      "li 5,0\n\t"
                      "li 6,0\n\t"
                      "mtctr %4\n"
                      "0:\tb 1f\n\t"
                      "addi 5,5,1\n"
                      "1:\tbdnz 0b\n\t"
                      "addi 6,6,1\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6\n\t"
                      "mfctr %2"
                      : "=&r"(r5), "=&r"(r6), "=&r"(ctr)
                      : "r"(zero), "r"(3ull)
                      : "r5", "r6", "ctr", CR_CLOBBERS);
    put(18, r5);
    put(20, r6);
    put(22, ctr);

    write_frame();
    return 0;
}
