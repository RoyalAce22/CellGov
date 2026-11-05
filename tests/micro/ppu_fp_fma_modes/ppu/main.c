/* PPU probe: the fused multiply-add forms, double and single, under each
 * FPSCR rounding mode.
 *
 * - which single-precision operands fmadds computes on, in each operand
 *   position, at and past each end of the single exponent range;
 * - one rounding of the exact result per mode, with a case where
 *   rounding the product first gives a different answer;
 * - the sign of an exact zero result, before and after the negation;
 * - the sign of a NaN result of the negated forms;
 * - overflow, tiny results, and a product cancelled to below the
 *   smallest denormal;
 * - FR and FI set before an exact result.
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
 *     +0   operation: 6 fmadd, 7 fmsub, 8 fnmadd, 9 fnmsub,
 *          15 fmadds, 18 fmsubs, 19 fnmadds, 20 fnmsubs
 *          (each computes FRA x FRC +/- FRB)
 *     +1   FPSCR low word before the instruction
 *     +2   FRA high, low    +4  FRB high, low    +6  FRC high, low
 *     +8   FRT high, low
 *     +10  FPSCR low word after the instruction
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define CASES 61
#define CASE_WORDS 11
#define WORDS (3 + CASE_WORDS * CASES)

#define OP_FMADD   6
#define OP_FMSUB   7
#define OP_FNMADD  8
#define OP_FNMSUB  9
#define OP_FMADDS  15
#define OP_FMSUBS  18
#define OP_FNMADDS 19
#define OP_FNMSUBS 20

#define SIGN    0x8000000000000000ull
#define ONE     0x3FF0000000000000ull
#define NEGONE  0xBFF0000000000000ull
#define TWO     0x4000000000000000ull
#define THREE   0x4008000000000000ull
#define HALF    0x3FE0000000000000ull
#define THIRD   0x3FD5555555555555ull /* the double nearest 1/3 */
#define ONEULP  0x3FF0000000000001ull /* 1 + 2^-52 */
#define INF     0x7FF0000000000000ull
#define NEGINF  0xFFF0000000000000ull
#define NEGZERO 0x8000000000000000ull
#define QNAN    0x7FF8000000000AAAull
#define SNAN    0x7FF0000000000BBBull

#define P127    0x47E0000000000000ull /* 2^127 */
#define P128    0x47F0000000000000ull /* 2^128 */
#define SMAX    0x47EFFFFFE0000000ull /* largest single */
#define M126    0x3810000000000000ull /* 2^-126 */
#define M149    0x36A0000000000000ull /* 2^-149 */
#define M150    0x3690000000000000ull /* 2^-150 */
#define DMIN    0x0010000000000000ull /* 2^-1022 */
#define DDEN    0x0000000000000001ull /* 2^-1074 */
#define P200    0x4C70000000000000ull /* 2^200 */
#define M200    0x3370000000000000ull /* 2^-200 */
#define DMAX    0x7FEFFFFFFFFFFFFFull

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

/* Each takes FRA, FRB, FRC and computes FRA x FRC +/- FRB. */
#define FMADD(pre, a, b, c)   CASE(OP_FMADD,   "fmadd %4,%1,%3,%2",   pre, a, b, c)
#define FMSUB(pre, a, b, c)   CASE(OP_FMSUB,   "fmsub %4,%1,%3,%2",   pre, a, b, c)
#define FNMADD(pre, a, b, c)  CASE(OP_FNMADD,  "fnmadd %4,%1,%3,%2",  pre, a, b, c)
#define FNMSUB(pre, a, b, c)  CASE(OP_FNMSUB,  "fnmsub %4,%1,%3,%2",  pre, a, b, c)
#define FMADDS(pre, a, b, c)  CASE(OP_FMADDS,  "fmadds %4,%1,%3,%2",  pre, a, b, c)
#define FMSUBS(pre, a, b, c)  CASE(OP_FMSUBS,  "fmsubs %4,%1,%3,%2",  pre, a, b, c)
#define FNMADDS(pre, a, b, c) CASE(OP_FNMADDS, "fnmadds %4,%1,%3,%2", pre, a, b, c)
#define FNMSUBS(pre, a, b, c) CASE(OP_FNMSUBS, "fnmsubs %4,%1,%3,%2", pre, a, b, c)

static void clear_fpscr(void)
{
    double f;
    opnd[0] = 0;
    __asm__ volatile ("lfd %0,0(%1)\n\tmtfsf 0xFF,%0"
                      : "=&f"(f) : "b"(opnd) : "memory");
}

/* Cases 0-14: one single-range edge as FRA, as FRC and as FRB. */
static void range_edge(unsigned long long x)
{
    FMADDS(CLEAN, x, 0, ONE);
    FMADDS(CLEAN, ONE, 0, x);
    FMADDS(CLEAN, ONE, x, ONE);
}

static void cases(void)
{
    unsigned int k;

    range_edge(P128);                    /* 0-2 */
    range_edge(M150);                    /* 3-5 */
    range_edge(DMIN);                    /* 6-8 */
    range_edge(P127);                    /* 9-11 */
    range_edge(M149);                    /* 12-14 */

    /* An out-of-range operand beside other classes. */
    FMADDS(CLEAN, INF, P200, ONE);       /* 15 */
    FMADDS(CLEAN, QNAN, P200, ONE);      /* 16 */
    FMADDS(CLEAN, 0, ONE, P200);         /* 17 */
    FMADDS(CLEAN, P200, NEGONE, M200);   /* 18 */

    /* One rounding per mode. */
    for (k = 0; k < 4; k++)
        FMADD(RN(k), ONEULP, NEGONE, ONEULP);   /* 19-22 */
    for (k = 0; k < 4; k++)
        FNMSUB(RN(k), ONEULP, ONE, ONEULP);     /* 23-26 */
    for (k = 0; k < 4; k++)
        FMADDS(RN(k), 0x3FF0000020000000ull, NEGONE,
               0x3FF0000020000000ull);          /* 27-30 */
    for (k = 0; k < 4; k++)
        FMSUB(RN(k), THREE, ONE, THIRD);        /* 31-34 */

    /* (1 + 2^-27)^2 - (1 + 2^-26) is 2^-54; rounding the product first
     * would give 0. */
    FMADD(RN(0), 0x3FF0000002000000ull, 0xBFF0000004000000ull,
          0x3FF0000002000000ull);        /* 35 */
    FNMADD(RN(0), 0x3FF0000002000000ull, 0xBFF0000004000000ull,
           0x3FF0000002000000ull);       /* 36 */

    /* The sign of an exact zero result. */
    FMADD(RN(3), ONE, NEGONE, ONE);      /* 37 */
    FNMADD(RN(3), ONE, NEGONE, ONE);     /* 38 */
    FMSUB(RN(3), ONE, ONE, ONE);         /* 39 */
    FNMSUB(RN(3), ONE, ONE, ONE);        /* 40 */
    FMADD(RN(0), NEGZERO, 0, ONE);       /* 41 */
    FMADD(RN(0), NEGZERO, NEGZERO, ONE); /* 42 */

    /* NaN results of the negated forms. */
    FNMADD(CLEAN, QNAN | SIGN, ONE, ONE);   /* 43 */
    FNMADD(CLEAN, INF, NEGINF, ONE);        /* 44 */
    FNMADDS(CLEAN, INF, NEGINF, ONE);       /* 45 */
    FNMSUB(CLEAN, ONE, SNAN, ONE);          /* 46 */

    /* Overflow, tiny results, and a cancellation below the denormals. */
    for (k = 0; k < 4; k++)
        FMADD(RN(k), DMAX, 0, TWO);      /* 47-50 */
    FMADD(RN(0), DMAX, DMAX, ONE);       /* 51 */
    FMADDS(RN(0), SMAX, 0, TWO);         /* 52 */
    FMADD(RN(0), DMIN, 0, HALF);         /* 53 exact */
    FMADD(RN(0), DMIN, DDEN, THIRD);     /* 54 inexact */
    FMADDS(RN(0), M126, 0, THIRD);       /* 55 */
    FMSUB(RN(0), 0x2000000000000001ull, 0x0010000000000002ull,
          0x2000000000000001ull);        /* 56 */
    FMSUB(RN(2), 0x2000000000000001ull, 0x0010000000000002ull,
          0x2000000000000001ull);        /* 57 */

    /* FR and FI set before an exact result. */
    FMADD(FRFIXX, ONE, ONE, ONE);        /* 58 */

    /* The other single forms. */
    FMSUBS(RN(0), THREE, ONE, THIRD);    /* 59 */
    FNMADDS(RN(0), ONE, ONE, ONE);       /* 60 */
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
