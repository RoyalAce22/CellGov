/* PPU probe: the FPSCR move instructions (mffs, mcrfs, mtfsfi, mtfsf,
 * mtfsb0, mtfsb1) and the CR1 copy of a floating-point record form.
 *
 * No step sets an exception enable bit (FPSCR bits 24:28), so no
 * exception can be enabled. Each step after the first starts from an
 * FPSCR cleared by mtfsf 0xFF unless it says otherwise. The probe
 * clears the FPSCR again before it exits.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code
 *   2, 3   mffs at entry into an FRT holding 0x1234567811223344 (high, low)
 *   4, 5   mffs again into an FRT holding all ones (high, low)
 *   6, 7   mffs after mtfsf 0xFF from zero (high, low)
 *   8      FPSCR after mtfsfi 0,15
 *   9      FPSCR after that and mtfsb0 0
 *   10     FPSCR after that and mtfsb1 3 (OX already 1)
 *   11     FPSCR after a clear and mtfsb1 23 (VXCVI)
 *   12     FPSCR after that and mtfsb0 2 (VX)
 *   13     FPSCR after that and mtfsb1 1 (FEX)
 *   14, 15 CR, FPSCR after that and mcrfs 5,0 (CR cleared first)
 *   16, 17 CR, FPSCR after that and mcrfs 4,5 (CR cleared first)
 *   18     FPSCR after a clear and mtfsfi 3,15
 *   19, 20 CR, FPSCR after that and mcrfs 0,3 (CR cleared first)
 *   21     FPSCR after a clear and mtfsf 0x80 from 0xFFFFFFFF_F0000000
 *   22     FPSCR after that and mtfsf 0x01 from 0x00000000_00000002
 *   23     FPSCR after a clear and mtfsf 0xFF from 0x00000000_60000000
 *   24     FPSCR after a clear and mtfsb1 13 (FR)
 *   25     CR after a clear, CR cleared, and mtfsb1. 3
 *   26     CR after a clear, CR cleared, and mtfsb1 3 (Rc=0)
 *   27, 28 CR, FPSCR after mtfsfi 0,9 (FX, OX), CR cleared, and
 *          fadd. 1.5 + 2.0
 *   29     CR after the same with fadd (Rc=0)
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 30

#define ALL_CR "cr0", "cr1", "cr2", "cr3", "cr4", "cr5", "cr6", "cr7"

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned long long opnd __attribute__((aligned(8)));
static volatile unsigned long long opnd2 __attribute__((aligned(8)));
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

/* mffs into an FRT that first holds `pre`; returns all 64 bits. */
static unsigned long long mffs_over(unsigned long long pre)
{
    double f;
    opnd = pre;
    __asm__ volatile ("lfd %0,0(%1)\n\tmffs %0\n\tstfd %0,0(%2)"
                      : "=&f"(f) : "b"(&opnd), "b"(&fout) : "memory");
    return fout;
}

static unsigned int fpscr(void)
{
    return (unsigned int)mffs_over(0);
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

/* CR after clearing it and running fadd(.) on 1.5 and 2.0. */
#define FADD_CR(out, insn)                                                   \
    do {                                                                     \
        double fa, fb, ft;                                                   \
        opnd = 0x3FF8000000000000ull;                                        \
        opnd2 = 0x4000000000000000ull;                                       \
        __asm__ volatile ("lfd %1,0(%4)\n\tlfd %2,0(%5)\n\t"                 \
                          "mtcrf 0xFF,%6\n\t" insn " %3,%1,%2\n\tmfcr %0"    \
                          : "=&r"(out), "=&f"(fa), "=&f"(fb), "=&f"(ft)      \
                          : "b"(&opnd), "b"(&opnd2), "r"(0)                  \
                          : "memory", ALL_CR);                               \
    } while (0)

int main(void)
{
    unsigned long long v;
    unsigned int cr;

    memset((void *)result, 0, sizeof(result));

    v = mffs_over(0x1234567811223344ull);
    result[2] = (unsigned int)(v >> 32);
    result[3] = (unsigned int)v;
    v = mffs_over(0xFFFFFFFFFFFFFFFFull);
    result[4] = (unsigned int)(v >> 32);
    result[5] = (unsigned int)v;

    CLEAR();
    v = mffs_over(0x1234567811223344ull);
    result[6] = (unsigned int)(v >> 32);
    result[7] = (unsigned int)v;

    FPSCR_OP("mtfsfi 0,15");
    result[8] = fpscr();
    FPSCR_OP("mtfsb0 0");
    result[9] = fpscr();
    FPSCR_OP("mtfsb1 3");
    result[10] = fpscr();

    CLEAR();
    FPSCR_OP("mtfsb1 23");
    result[11] = fpscr();
    FPSCR_OP("mtfsb0 2");
    result[12] = fpscr();
    FPSCR_OP("mtfsb1 1");
    result[13] = fpscr();
    CR_AFTER(cr, "mcrfs 5,0");
    result[14] = cr;
    result[15] = fpscr();
    CR_AFTER(cr, "mcrfs 4,5");
    result[16] = cr;
    result[17] = fpscr();

    CLEAR();
    FPSCR_OP("mtfsfi 3,15");
    result[18] = fpscr();
    CR_AFTER(cr, "mcrfs 0,3");
    result[19] = cr;
    result[20] = fpscr();

    CLEAR();
    MTFSF(0x80, 0xFFFFFFFFF0000000ull);
    result[21] = fpscr();
    MTFSF(0x01, 0x0000000000000002ull);
    result[22] = fpscr();

    CLEAR();
    MTFSF(0xFF, 0x0000000060000000ull);
    result[23] = fpscr();

    CLEAR();
    FPSCR_OP("mtfsb1 13");
    result[24] = fpscr();

    CLEAR();
    CR_AFTER(cr, "mtfsb1. 3");
    result[25] = cr;
    CLEAR();
    CR_AFTER(cr, "mtfsb1 3");
    result[26] = cr;

    CLEAR();
    FPSCR_OP("mtfsfi 0,9");
    FADD_CR(cr, "fadd.");
    result[27] = cr;
    result[28] = fpscr();
    CLEAR();
    FPSCR_OP("mtfsfi 0,9");
    FADD_CR(cr, "fadd");
    result[29] = cr;

    CLEAR();
    write_frame();
    return 0;
}
