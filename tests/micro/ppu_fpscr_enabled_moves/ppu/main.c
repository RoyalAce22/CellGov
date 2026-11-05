/* PPU probe: whether an FPSCR move instruction that sets an exception
 * bit and its enable bit together raises an interrupt.
 *
 * Each step clears the FPSCR with mtfsf 0xFF, then sets an exception bit
 * and its enable with the move instructions, and stores the FPSCR. FEX
 * reads 1 after every step. The process reaches its end and writes the
 * frame only if no step raised an interrupt that ended it; word 1 counts
 * the steps that ran. The probe clears the FPSCR again before it exits.
 * No system call other than the TTY and result-file writes.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  steps that ran (7)
 *   2  FPSCR after mtfsf 0xFF from 0x04000010 (ZX, ZE)
 *   3  FPSCR after mtfsf 0xFF from 0x01000080 (VXSNAN, VE)
 *   4  FPSCR after mtfsb1 5 (ZX), then mtfsfi 6,1 (ZE)
 *   5  FPSCR after mtfsfi 6,1 (ZE), then mtfsb1 5 (ZX)
 *   6  FPSCR after mtfsb1 28 (XE), then mtfsb1 6 (XX)
 *   7  FPSCR after mtfsb1 25 (OE), then mtfsb1 3 (OX)
 *   8  FPSCR after mtfsb1 26 (UE), then mtfsb1 4 (UX)
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 9
#define STEPS 7

#define ALL_CR "cr0", "cr1", "cr2", "cr3", "cr4", "cr5", "cr6", "cr7"

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned long long opnd __attribute__((aligned(8)));
static volatile unsigned long long fout __attribute__((aligned(8)));

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

static unsigned int fpscr(void)
{
    double f;
    __asm__ volatile ("mffs %0\n\tstfd %0,0(%1)"
                      : "=&f"(f) : "b"(&fout) : "memory");
    return (unsigned int)fout;
}

/* mtfsf FLM from the 64-bit value `v`. FLM must be a literal. */
#define MTFSF(flm, v)                                                        \
    do {                                                                     \
        double f;                                                            \
        opnd = (v);                                                          \
        __asm__ volatile ("lfd %0,0(%1)\n\tmtfsf " #flm ",%0"                \
                          : "=&f"(f) : "b"(&opnd) : "memory");               \
    } while (0)

#define CLEAR() MTFSF(0xFF, 0ull)

/* One FPSCR instruction with no register operand. */
#define FPSCR_OP(text) __asm__ volatile (text ::: "memory", ALL_CR)

int main(void)
{
    memset((void *)result, 0, sizeof(result));

    CLEAR();
    MTFSF(0xFF, 0x0000000004000010ull);
    result[2] = fpscr();
    result[1]++;

    CLEAR();
    MTFSF(0xFF, 0x0000000001000080ull);
    result[3] = fpscr();
    result[1]++;

    CLEAR();
    FPSCR_OP("mtfsb1 5");
    FPSCR_OP("mtfsfi 6,1");
    result[4] = fpscr();
    result[1]++;

    CLEAR();
    FPSCR_OP("mtfsfi 6,1");
    FPSCR_OP("mtfsb1 5");
    result[5] = fpscr();
    result[1]++;

    CLEAR();
    FPSCR_OP("mtfsb1 28");
    FPSCR_OP("mtfsb1 6");
    result[6] = fpscr();
    result[1]++;

    CLEAR();
    FPSCR_OP("mtfsb1 25");
    FPSCR_OP("mtfsb1 3");
    result[7] = fpscr();
    result[1]++;

    CLEAR();
    FPSCR_OP("mtfsb1 26");
    FPSCR_OP("mtfsb1 4");
    result[8] = fpscr();
    result[1]++;

    CLEAR();
    if (result[1] != STEPS)
        result[0] = 1;
    write_frame();
    return 0;
}
