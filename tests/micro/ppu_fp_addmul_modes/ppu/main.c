/* PPU probe: fadd, fsub and fmul, double and single, under each FPSCR
 * rounding mode, and the single-precision arms on operands single format
 * cannot hold.
 *
 * - which single-precision operands give the default QNaN: values at
 *   and past each end of the single exponent range, in each operand
 *   position, and with a NaN, an infinity or a zero beside them;
 * - x - x and the exact zero sums in each mode;
 * - round and sticky combinations with and without a carry, per mode;
 * - overflow per mode and sign, both widths;
 * - tiny exact and tiny inexact results, both widths;
 * - a NaN in the reserved FRB of fmul and FRC of fadd.
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

#define CASES 97
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
#define INF     0x7FF0000000000000ull
#define NEGZERO 0x8000000000000000ull
#define QNAN    0x7FF8000000000AAAull
#define SNAN    0x7FF0000000000BBBull

/* Single-range edges, in double format. */
#define P127    0x47E0000000000000ull /* 2^127 */
#define P128    0x47F0000000000000ull /* 2^128 */
#define SMAX    0x47EFFFFFE0000000ull /* largest single */
#define SMAXP   0x47EFFFFFF0000000ull /* half a single ulp above it */
#define M126    0x3810000000000000ull /* 2^-126 */
#define M127    0x3800000000000000ull /* 2^-127 */
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

/* Cases 0-39: one single-range edge in each operand position. */
static void range_edge(unsigned long long x)
{
    FADDS(CLEAN, x, ONE);
    FADDS(CLEAN, ONE, x);
    FMULS(CLEAN, x, ONE);
    FMULS(CLEAN, ONE, x);
}

static void cases(void)
{
    unsigned int k;

    range_edge(P127);                    /* 0-3 */
    range_edge(P128);                    /* 4-7 */
    range_edge(SMAX);                    /* 8-11 */
    range_edge(SMAXP);                   /* 12-15 */
    range_edge(M126);                    /* 16-19 */
    range_edge(M127);                    /* 20-23 */
    range_edge(M149);                    /* 24-27 */
    range_edge(M150);                    /* 28-31 */
    range_edge(DMIN);                    /* 32-35 */
    range_edge(DDEN);                    /* 36-39 */

    /* An out-of-range operand beside other classes. */
    FADDS(CLEAN, P128 | SIGN, ONE);      /* 40 */
    FADDS(CLEAN, INF, P200);             /* 41 */
    FADDS(CLEAN, QNAN, P200);            /* 42 */
    FADDS(CLEAN, P200, SNAN);            /* 43 */
    FMULS(CLEAN, 0, P200);               /* 44 */
    FSUBS(CLEAN, P200, P200);            /* 45 */
    FADDS(FRFIXX, P200, ONE);            /* 46 */
    FADDS(CLEAN, 0x3FF0000000000001ull, NEGONE); /* 47 wide fraction */

    /* x - x and the exact zero sums. */
    for (k = 0; k < 4; k++)
        FSUB(RN(k), ONE, ONE);           /* 48-51 */
    for (k = 0; k < 4; k++)
        FSUBS(RN(k), ONE, ONE);          /* 52-55 */
    FADD(RN(3), NEGZERO, 0);             /* 56 */
    FADD(RN(0), NEGZERO, 0);             /* 57 */
    FADD(RN(3), NEGZERO, NEGZERO);       /* 58 */

    /* Just above half an ulp, each sign, each mode; then the two ties. */
    for (k = 0; k < 4; k++)
        FADD(RN(k), ONE, 0x3CA0000000000001ull);           /* 59-62 */
    for (k = 0; k < 4; k++)
        FADD(RN(k), NEGONE, 0xBCA0000000000001ull);        /* 63-66 */
    FADD(RN(0), ONE, 0x3CA0000000000000ull);               /* 67 */
    FADD(RN(0), 0x3FF0000000000001ull, 0x3CA0000000000000ull); /* 68 */
    for (k = 0; k < 4; k++)
        FADDS(RN(k), ONE, 0x3E70000000000001ull);          /* 69-72 */

    /* Overflow per mode and sign. */
    for (k = 0; k < 4; k++)
        FMUL(RN(k), DMAX, TWO);          /* 73-76 */
    for (k = 0; k < 4; k++)
        FMUL(RN(k), DMAX | SIGN, TWO);   /* 77-80 */
    for (k = 0; k < 4; k++)
        FMULS(RN(k), SMAX, TWO);         /* 81-84 */
    FADD(RN(0), DMAX, DMAX);             /* 85 */

    /* Tiny results, exact and inexact. */
    FMUL(RN(0), DMIN, 0x3FE8000000000000ull);   /* 86 exact */
    FMUL(RN(0), DMIN, 0x3E10000000000001ull);   /* 87 inexact */
    FMULS(RN(0), M126, 0x3F50000000000000ull);  /* 88 exact */
    FMULS(RN(0), M126, 0x3F50000020000000ull);  /* 89 inexact */
    FMUL(RN(1), DMIN, 0x3E10000000000001ull);   /* 90 */

    /* A NaN in a reserved field. */
    CASE(OP_FMUL, "fmul %4,%1,%3", CLEAN, ONE, SNAN, TWO); /* 91 */
    CASE(OP_FADD, "fadd %4,%1,%2", CLEAN, ONE, TWO, SNAN); /* 92 */

    /* A product with bits below the double precision, per mode. */
    for (k = 0; k < 4; k++)
        FMUL(RN(k), 0x3FF0000000000001ull, 0x3FF0000000000001ull); /* 93-96 */
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
