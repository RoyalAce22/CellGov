/* PPU probe: fadd, fsub and fmul, double and single, on the operand
 * classes the ppu_fp_addmul_modes probe does not send.
 *
 * - a NaN only in FRB of fsub, of each sign, quiet and signaling;
 * - an infinity of either sign in either operand of each arm;
 * - denormal operands of the double arms, exact, inexact and tied;
 * - sums whose smaller operand is only a sticky bit, in each mode;
 * - signed zero products from FR, FI and XX set;
 * - a single-precision operand out of single range beside an infinity,
 *   a NaN or a zero, and in FRB of fsubs;
 * - the sign of 0 - 0 and its mixed-sign forms.
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
 *     +0   operation: 1 fadd, 2 fsub, 3 fmul, 10 fadds, 11 fsubs,
 *          12 fmuls
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

#define CASES 63
#define CASE_WORDS 11
#define WORDS (3 + CASE_WORDS * CASES)

#define OP_FADD    1
#define OP_FSUB    2
#define OP_FMUL    3
#define OP_FADDS   10
#define OP_FSUBS   11
#define OP_FMULS   12

#define SIGN    0x8000000000000000ull
#define ONE     0x3FF0000000000000ull
#define NEGONE  0xBFF0000000000000ull
#define TWO     0x4000000000000000ull
#define HALF    0x3FE0000000000000ull
#define INF     0x7FF0000000000000ull
#define NEGINF  0xFFF0000000000000ull
#define NEGZERO 0x8000000000000000ull
#define QNAN    0x7FF8000000000AAAull
#define SNAN    0x7FF0000000000BBBull

#define DMIN    0x0010000000000000ull /* 2^-1022 */
#define DDEN    0x0000000000000001ull /* 2^-1074 */
#define DSUBMAX 0x000FFFFFFFFFFFFFull /* largest double denormal */
#define M100    0x39B0000000000000ull /* 2^-100 */
#define M150    0x3690000000000000ull /* 2^-150 */
#define M200    0x3370000000000000ull /* 2^-200 */
#define P200    0x4C70000000000000ull /* 2^200 */

/* FPSCR starting values. */
#define CLEAN    0x00000000u
#define FRFIXX   0x02060000u /* XX, FR, FI */
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

#define FADD(pre, a, b)   CASE(OP_FADD,  "fadd %4,%1,%2",  pre, a, b, 0)
#define FSUB(pre, a, b)   CASE(OP_FSUB,  "fsub %4,%1,%2",  pre, a, b, 0)
#define FMUL(pre, a, c)   CASE(OP_FMUL,  "fmul %4,%1,%3",  pre, a, 0, c)
#define FADDS(pre, a, b)  CASE(OP_FADDS, "fadds %4,%1,%2", pre, a, b, 0)
#define FSUBS(pre, a, b)  CASE(OP_FSUBS, "fsubs %4,%1,%2", pre, a, b, 0)
#define FMULS(pre, a, c)  CASE(OP_FMULS, "fmuls %4,%1,%3", pre, a, 0, c)

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

    /* A NaN only in FRB of fsub. */
    FSUB(CLEAN, ONE, QNAN);              /* 0 */
    FSUB(CLEAN, ONE, QNAN | SIGN);       /* 1 */
    FSUB(CLEAN, ONE, SNAN);              /* 2 */
    FSUB(CLEAN, ONE, SNAN | SIGN);       /* 3 */
    FSUBS(CLEAN, ONE, QNAN | SIGN);      /* 4 */
    FSUB(CLEAN, QNAN, SNAN);             /* 5 */
    FSUB(CLEAN, INF, QNAN | SIGN);       /* 6 */

    /* Infinite products. */
    FMUL(CLEAN, NEGINF, ONE);            /* 7 */
    FMUL(CLEAN, ONE, NEGINF);            /* 8 */
    FMUL(CLEAN, NEGINF, NEGONE);         /* 9 */
    FMUL(CLEAN, INF, NEGONE);            /* 10 */
    FMUL(FRFIXX, NEGINF, DDEN);          /* 11 */
    FMULS(CLEAN, NEGINF, TWO);           /* 12 */

    /* Infinite sums. */
    FADD(CLEAN, NEGINF, ONE);            /* 13 */
    FADD(CLEAN, ONE, NEGINF);            /* 14 */
    FSUB(CLEAN, ONE, INF);               /* 15 */
    FSUB(CLEAN, ONE, NEGINF);            /* 16 */
    FSUB(CLEAN, NEGINF, ONE);            /* 17 */
    FADDS(CLEAN, ONE, NEGINF);           /* 18 */
    FSUBS(CLEAN, ONE, INF);              /* 19 */

    /* Denormal operands of the double arms. */
    FADD(CLEAN, DDEN, DDEN);             /* 20 */
    FADD(RN(0), DDEN, ONE);              /* 21 */
    FADD(RN(2), DDEN, ONE);              /* 22 */
    FMUL(CLEAN, DDEN, TWO);              /* 23 */
    FMUL(CLEAN, DSUBMAX, TWO);           /* 24 */
    FSUB(CLEAN, DMIN, DDEN);             /* 25 */
    FMUL(RN(0), DDEN, HALF);             /* 26 */
    FMUL(RN(2), DDEN, HALF);             /* 27 */
    FSUB(RN(3), DDEN, DDEN);             /* 28 */

    /* The smaller operand is only a sticky bit, each mode. */
    for (k = 0; k < 4; k++)
        FADD(RN(k), ONE, M200);          /* 29-32 */
    for (k = 0; k < 4; k++)
        FADD(RN(k), NEGONE, M200);       /* 33-36 */
    for (k = 0; k < 4; k++)
        FSUB(RN(k), ONE, M200);          /* 37-40 */
    for (k = 0; k < 4; k++)
        FADDS(RN(k), ONE, M100);         /* 41-44 */

    /* Signed zero products from FR, FI and XX set. */
    FMUL(FRFIXX, NEGZERO, ONE);          /* 45 */
    FMUL(FRFIXX, 0, NEGONE);             /* 46 */
    FMUL(FRFIXX, NEGZERO, NEGZERO);      /* 47 */
    FMULS(FRFIXX, NEGZERO, ONE);         /* 48 */
    FMUL(FRFIXX, 0, DDEN);               /* 49 */

    /* A single-precision operand out of single range beside other
     * classes, and in FRB of fsubs. */
    FMULS(CLEAN, INF, P200);             /* 50 */
    FMULS(CLEAN, P200, NEGINF);          /* 51 */
    FSUBS(CLEAN, P200, INF);             /* 52 */
    FMULS(CLEAN, QNAN, M150);            /* 53 */
    FMULS(CLEAN, P200, SNAN);            /* 54 */
    FSUBS(CLEAN, ONE, P200);             /* 55 */
    FSUBS(CLEAN, ONE, M150);             /* 56 */
    FMULS(CLEAN, INF, M150);             /* 57 */
    FMULS(CLEAN, NEGZERO, M150);         /* 58 */

    /* The sign of 0 - 0. */
    FSUB(RN(3), 0, 0);                   /* 59 */
    FSUB(RN(0), 0, 0);                   /* 60 */
    FSUB(RN(0), NEGZERO, 0);             /* 61 */
    FSUB(RN(0), 0, NEGZERO);             /* 62 */
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
