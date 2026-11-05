/* PPU probe: fdiv, fdivs, fsqrt and fsqrts under each FPSCR rounding
 * mode, their special operands, and the single-precision forms on
 * operands single format cannot hold.
 *
 * - which single-precision operands fdivs and fsqrts compute on, and how
 *   they round, at and past each end of the single exponent range;
 * - rounding per mode and sign, both widths;
 * - zero divide, 0/0, inf/inf, signed zeros and infinities, NaNs;
 * - the square root of -0, -inf, a negative number, denormals and NaNs;
 * - overflow and tiny results, both widths;
 * - FR and FI set before an exact result;
 * - a NaN in a reserved field.
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
 *     +0   operation: 4 fdiv, 5 fsqrt, 13 fdivs, 14 fsqrts
 *     +1   FPSCR low word before the instruction
 *     +2   FRA high, low    +4  FRB high, low    +6  FRC high, low
 *          (an operand the operation does not read is 0 unless the
 *          case puts a NaN there)
 *     +8   FRT high, low
 *     +10  FPSCR low word after the instruction
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define CASES 85
#define CASE_WORDS 11
#define WORDS (3 + CASE_WORDS * CASES)

#define OP_FDIV    4
#define OP_FSQRT   5
#define OP_FDIVS   13
#define OP_FSQRTS  14

#define SIGN    0x8000000000000000ull
#define ONE     0x3FF0000000000000ull
#define NEGONE  0xBFF0000000000000ull
#define TWO     0x4000000000000000ull
#define THREE   0x4008000000000000ull
#define HALF    0x3FE0000000000000ull
#define INF     0x7FF0000000000000ull
#define NEGINF  0xFFF0000000000000ull
#define NEGZERO 0x8000000000000000ull
#define QNAN    0x7FF8000000000AAAull
#define SNAN    0x7FF0000000000BBBull

/* Single-range edges, in double format. */
#define P127    0x47E0000000000000ull /* 2^127 */
#define P128    0x47F0000000000000ull /* 2^128 */
#define SMAX    0x47EFFFFFE0000000ull /* largest single */
#define M126    0x3810000000000000ull /* 2^-126 */
#define M149    0x36A0000000000000ull /* 2^-149 */
#define M150    0x3690000000000000ull /* 2^-150 */
#define DMIN    0x0010000000000000ull /* 2^-1022 */
#define DDEN    0x0000000000000001ull /* 2^-1074 */
#define P200    0x4C70000000000000ull /* 2^200 */
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

#define FDIV(pre, a, b)   CASE(OP_FDIV,   "fdiv %4,%1,%2",  pre, a, b, 0)
#define FDIVS(pre, a, b)  CASE(OP_FDIVS,  "fdivs %4,%1,%2", pre, a, b, 0)
#define FSQRT(pre, b)     CASE(OP_FSQRT,  "fsqrt %4,%2",    pre, 0, b, 0)
#define FSQRTS(pre, b)    CASE(OP_FSQRTS, "fsqrts %4,%2",   pre, 0, b, 0)

static void clear_fpscr(void)
{
    double f;
    opnd[0] = 0;
    __asm__ volatile ("lfd %0,0(%1)\n\tmtfsf 0xFF,%0"
                      : "=&f"(f) : "b"(opnd) : "memory");
}

/* Cases 0-17: one single-range edge as dividend, divisor and root. */
static void range_edge(unsigned long long x)
{
    FDIVS(CLEAN, x, ONE);
    FDIVS(CLEAN, ONE, x);
    FSQRTS(CLEAN, x);
}

static void cases(void)
{
    unsigned int k;

    range_edge(P128);                    /* 0-2 */
    range_edge(M150);                    /* 3-5 */
    range_edge(DMIN);                    /* 6-8 */
    range_edge(DDEN);                    /* 9-11 */
    range_edge(P127);                    /* 12-14 */
    range_edge(M149);                    /* 15-17 */

    /* An out-of-range operand with an inexact result, and beside other
     * classes. */
    FDIVS(CLEAN, 0x47F0000000000001ull, THREE); /* 18 */
    FDIVS(CLEAN, P200, THREE);           /* 19 */
    FSQRTS(CLEAN, 0x47F0000000000001ull); /* 20 */
    FDIVS(CLEAN, ONE, THREE);            /* 21 */
    FDIVS(CLEAN, SNAN, P200);            /* 22 */
    FDIVS(CLEAN, P200, 0);               /* 23 */
    FDIVS(CLEAN, 0, P200);               /* 24 */
    FDIVS(CLEAN, INF, P200);             /* 25 */

    /* Rounding per mode and sign. */
    for (k = 0; k < 4; k++)
        FDIV(RN(k), ONE, THREE);         /* 26-29 */
    for (k = 0; k < 4; k++)
        FDIV(RN(k), NEGONE, THREE);      /* 30-33 */
    for (k = 0; k < 4; k++)
        FDIVS(RN(k), ONE, THREE);        /* 34-37 */
    for (k = 0; k < 4; k++)
        FSQRT(RN(k), TWO);               /* 38-41 */
    for (k = 0; k < 4; k++)
        FSQRTS(RN(k), TWO);              /* 42-45 */

    /* Divide specials. */
    FDIV(CLEAN, 0, 0);                   /* 46 */
    FDIV(CLEAN, INF, INF);               /* 47 */
    FDIV(CLEAN, ONE, 0);                 /* 48 */
    FDIV(CLEAN, NEGONE, 0);              /* 49 */
    FDIV(CLEAN, ONE, NEGZERO);           /* 50 */
    FDIV(CLEAN, 0, ONE);                 /* 51 */
    FDIV(CLEAN, NEGZERO, ONE);           /* 52 */
    FDIV(CLEAN, ONE, INF);               /* 53 */
    FDIV(CLEAN, NEGONE, INF);            /* 54 */
    FDIV(CLEAN, INF, NEGONE);            /* 55 */
    FDIV(CLEAN, SNAN, 0);                /* 56 */
    FDIV(CLEAN, QNAN, SNAN);             /* 57 */
    FDIV(CLEAN, ONE, QNAN);              /* 58 */

    /* Square-root specials. */
    FSQRT(CLEAN, NEGZERO);               /* 59 */
    FSQRT(CLEAN, 0);                     /* 60 */
    FSQRT(CLEAN, INF);                   /* 61 */
    FSQRT(CLEAN, NEGINF);                /* 62 */
    FSQRT(CLEAN, NEGONE);                /* 63 */
    FSQRT(CLEAN, 0x8000000000000001ull); /* 64 */
    FSQRT(CLEAN, SNAN);                  /* 65 */
    FSQRT(CLEAN, QNAN | SIGN);           /* 66 */

    /* Overflow and tiny results. */
    for (k = 0; k < 4; k++)
        FDIV(RN(k), DMAX, HALF);         /* 67-70 */
    FDIV(RN(0), DMIN, TWO);              /* 71 exact */
    FDIV(RN(0), DMIN, THREE);            /* 72 inexact */
    FDIVS(RN(0), SMAX, HALF);            /* 73 */
    FDIVS(RN(0), M126, THREE);           /* 74 */
    FSQRT(RN(0), DDEN);                  /* 75 exact */
    FSQRT(RN(0), 0x0000000000000002ull); /* 76 inexact */

    /* FR and FI set before an exact result. */
    FDIV(FRFIXX, ONE, TWO);              /* 77 */
    FSQRT(FRFIXX, 0x4010000000000000ull); /* 78 sqrt 4 */

    /* A NaN in a reserved field. */
    CASE(OP_FDIV, "fdiv %4,%1,%2", CLEAN, ONE, TWO, SNAN);     /* 79 */
    CASE(OP_FSQRT, "fsqrt %4,%2", CLEAN, SNAN, TWO, SNAN);     /* 80 */

    /* A root just above 1, per mode. */
    for (k = 0; k < 4; k++)
        FSQRT(RN(k), 0x3FF0000000000001ull); /* 81-84 */
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
