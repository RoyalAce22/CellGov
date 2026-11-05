/* PPU probe: fdivs and fsqrts on double operands inside the single
 * exponent range that carry fraction bits below single precision.
 *
 * - an operand one double ulp above or below 1, per rounding mode, as
 *   dividend, divisor and root;
 * - a dividend just above, at and just below the single halfway point;
 * - general double operands, per rounding mode;
 * - tiny results from such an operand, nearest and toward +infinity.
 *
 * A result that rounds the exact quotient or root of the full double
 * operands differs, in the directed modes, from one of operands first
 * cut to single.
 *
 * Each case sets the whole FPSCR to its starting value with mtfsf 0xFF
 * (every enable bit 0 in every case; RN as the case names), loads its
 * operands from memory, runs one instruction, stores the result and then
 * the mffs result. The probe clears the FPSCR before it exits. No system
 * call other than the TTY and result-file writes.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code   2  case count
 *   3 + 11k, case k:
 *     +0   operation: 13 fdivs, 14 fsqrts
 *     +1   FPSCR low word before the instruction
 *     +2   FRA high, low    +4  FRB high, low    +6  FRC high, low
 *          (an operand the operation does not read is 0)
 *     +8   FRT high, low
 *     +10  FPSCR low word after the instruction
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define CASES 31
#define CASE_WORDS 11
#define WORDS (3 + CASE_WORDS * CASES)

#define OP_FDIVS   13
#define OP_FSQRTS  14

#define ONE     0x3FF0000000000000ull

/* In-range operands with bits below single precision. */
#define UP1     0x3FF0000000000001ull /* 1 + 2^-52 */
#define DOWN1   0x3FEFFFFFFFFFFFFFull /* 1 - 2^-53 */
#define HALFUP  0x3FF0000010000001ull /* 1 + 2^-24 + 2^-52 */
#define HALFWAY 0x3FF0000010000000ull /* 1 + 2^-24 */
#define HALFDN  0x3FF000000FFFFFFFull /* 1 + 2^-24 - 2^-52 */
#define PI      0x400921FB54442D18ull
#define E       0x4005BF0A8B145769ull
#define TWOUP   0x4000000000000001ull /* 2 + 2^-51 */
#define TINYN   0x3810000000000001ull /* 2^-126 (1 + 2^-52) */
#define TINYD   0x3800000000000001ull /* 2^-127 (1 + 2^-52) */

#define RN(k)    ((unsigned int)(k))

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned long long opnd[4] __attribute__((aligned(8)));
static volatile unsigned long long fout[2] __attribute__((aligned(8)));
static unsigned int next_case;

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

static void put64(unsigned int at, unsigned long long v)
{
    result[at] = (unsigned int)(v >> 32);
    result[at + 1] = (unsigned int)v;
}

/* Records case `next_case` from opnd[] and fout[]. */
static void put_case(unsigned int op)
{
    unsigned int at = 3 + CASE_WORDS * next_case;
    if (next_case >= CASES)
        return;
    result[at] = op;
    result[at + 1] = (unsigned int)opnd[0];
    put64(at + 2, opnd[1]);
    put64(at + 4, opnd[2]);
    put64(at + 6, opnd[3]);
    put64(at + 8, fout[0]);
    result[at + 10] = (unsigned int)fout[1];
    next_case++;
}

/* Runs `insn` from FPSCR `pre` with FRA = %1, FRB = %2, FRC = %3 and
 * FRT = %4, then stores FRT and the FPSCR. */
#define CASE(op, insn, pre, a, b, c)                                         \
    do {                                                                     \
        double fm, fa, fb, fc, ft, fs;                                       \
        opnd[0] = (pre); opnd[1] = (a); opnd[2] = (b); opnd[3] = (c);        \
        __asm__ volatile ("lfd %0,0(%6)\n\tmtfsf 0xFF,%0\n\t"                \
                          "lfd %1,8(%6)\n\tlfd %2,16(%6)\n\t"                \
                          "lfd %3,24(%6)\n\t"                                \
                          insn "\n\tstfd %4,0(%7)\n\t"                       \
                          "mffs %5\n\tstfd %5,8(%7)"                         \
                          : "=&f"(fm), "=&f"(fa), "=&f"(fb), "=&f"(fc),      \
                            "=&f"(ft), "=&f"(fs)                             \
                          : "b"(opnd), "b"(fout) : "memory");                \
        put_case(op);                                                        \
    } while (0)

#define FDIVS(pre, a, b)  CASE(OP_FDIVS,  "fdivs %4,%1,%2", pre, a, b, 0)
#define FSQRTS(pre, b)    CASE(OP_FSQRTS, "fsqrts %4,%2",   pre, 0, b, 0)

static void clear_fpscr(void)
{
    double f;
    opnd[0] = 0;
    __asm__ volatile ("lfd %0,0(%1)\n\tmtfsf 0xFF,%0"
                      : "=&f"(f) : "b"(opnd) : "memory");
}

static void cases(void)
{
    unsigned int k;

    for (k = 0; k < 4; k++)
        FDIVS(RN(k), UP1, ONE);          /* 0-3 */
    for (k = 0; k < 4; k++)
        FDIVS(RN(k), ONE, UP1);          /* 4-7 */
    FDIVS(RN(0), HALFUP, ONE);           /* 8 */
    FDIVS(RN(0), HALFWAY, ONE);          /* 9 */
    FDIVS(RN(0), HALFDN, ONE);           /* 10 */
    for (k = 0; k < 4; k++)
        FDIVS(RN(k), PI, E);             /* 11-14 */
    for (k = 0; k < 4; k++)
        FSQRTS(RN(k), UP1);              /* 15-18 */
    for (k = 0; k < 4; k++)
        FSQRTS(RN(k), DOWN1);            /* 19-22 */
    FSQRTS(RN(0), TWOUP);                /* 23 */
    FDIVS(RN(0), TINYN, ONE);            /* 24 */
    FDIVS(RN(2), TINYN, ONE);            /* 25 */
    FDIVS(RN(0), TINYD, ONE);            /* 26 */
    FDIVS(RN(2), TINYD, ONE);            /* 27 */
    FSQRTS(RN(0), PI);                   /* 28 */
    FDIVS(RN(0), ONE, DOWN1);            /* 29 */
    FDIVS(RN(2), ONE, DOWN1);            /* 30 */
}

int main(void)
{
    memset((void *)result, 0, sizeof(result));
    next_case = 0;
    cases();
    clear_fpscr();
    result[2] = next_case;
    if (next_case != CASES) {
        result[0] = 1;
        result[1] = next_case;
    }
    write_frame();
    return 0;
}
