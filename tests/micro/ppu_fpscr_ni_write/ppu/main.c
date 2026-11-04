/* PPU probe: whether the FPSCR move instructions can set NI (bit 29),
 * the non-IEEE mode bit, and the reserved bit 20.
 *
 * No step sets an exception enable bit (FPSCR bits 24:28) together with
 * an exception bit, so no exception can be enabled. Each step starts
 * from an FPSCR cleared by mtfsf 0xFF unless it says otherwise. The
 * probe clears the FPSCR again before it exits.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code
 *   2      FPSCR after mtfsf 0xFF from 0x00000004 (NI)
 *   3      FPSCR after mtfsf 0xFF from 0x00000007 (NI, RN 3)
 *   4      FPSCR after mtfsf 0x01 from 0x0000000C (XE, NI; field 7 only)
 *   5      FPSCR after mtfsb1 29
 *   6      FPSCR after mtfsfi 7,4
 *   7      FPSCR after mtfsfi 7,15
 *   8      FPSCR after mtfsf 0xFF from 0x00000004, then mtfsb0 29
 *   9      FPSCR after mtfsb1 29, then mtfsb1 30
 *   10, 11 CR, FPSCR after mtfsfi 7,4, CR cleared, and mcrfs 7,7
 *   12     FPSCR after mtfsf 0xFF from 0x00000800 (bit 20)
 *   13     FPSCR after mtfsb1 20
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 14

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

/* CR after clearing it and running `text`. */
#define CR_AFTER(out, text)                                                  \
    __asm__ volatile ("mtcrf 0xFF,%1\n\t" text "\n\tmfcr %0"                 \
                      : "=&r"(out) : "r"(0) : "memory", ALL_CR)

int main(void)
{
    unsigned int cr;

    memset((void *)result, 0, sizeof(result));

    CLEAR();
    MTFSF(0xFF, 0x0000000000000004ull);
    result[2] = fpscr();

    CLEAR();
    MTFSF(0xFF, 0x0000000000000007ull);
    result[3] = fpscr();

    CLEAR();
    MTFSF(0x01, 0x000000000000000Cull);
    result[4] = fpscr();

    CLEAR();
    FPSCR_OP("mtfsb1 29");
    result[5] = fpscr();

    CLEAR();
    FPSCR_OP("mtfsfi 7,4");
    result[6] = fpscr();

    CLEAR();
    FPSCR_OP("mtfsfi 7,15");
    result[7] = fpscr();

    CLEAR();
    MTFSF(0xFF, 0x0000000000000004ull);
    FPSCR_OP("mtfsb0 29");
    result[8] = fpscr();

    CLEAR();
    FPSCR_OP("mtfsb1 29");
    FPSCR_OP("mtfsb1 30");
    result[9] = fpscr();

    CLEAR();
    FPSCR_OP("mtfsfi 7,4");
    CR_AFTER(cr, "mcrfs 7,7");
    result[10] = cr;
    result[11] = fpscr();

    CLEAR();
    MTFSF(0xFF, 0x0000000000000800ull);
    result[12] = fpscr();

    CLEAR();
    FPSCR_OP("mtfsb1 20");
    result[13] = fpscr();

    CLEAR();
    write_frame();
    return 0;
}
