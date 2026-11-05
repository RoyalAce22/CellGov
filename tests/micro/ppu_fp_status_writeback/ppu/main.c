/* PPU probe: the FPSCR status one arithmetic or estimate instruction
 * writes -- the invalid-operation causes of the multiply-add forms and of
 * the single-precision arms, ZX, FPRF, whether an invalid operation or a
 * zero divide clears FR and FI that an earlier rounding set, whether FX
 * is set again for an exception bit that is already set, and the status
 * of fres and frsqrte on operands that are not powers of two.
 *
 * Each case sets the whole FPSCR to its starting value with mtfsf 0xFF
 * (RN 0 and every enable bit 0 in every case), loads its operands from
 * memory, runs one instruction, stores the result and then the mffs
 * result. No case sets an exception enable bit. The probe clears the
 * FPSCR before it exits. No system call other than the TTY and
 * result-file writes.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code   2  case count
 *   3 + 11k, case k:
 *     +0   operation: 1 fadd, 2 fsub, 3 fmul, 4 fdiv, 5 fsqrt, 6 fmadd,
 *          7 fmsub, 8 fnmadd, 9 fnmsub, 10 fadds, 11 fsubs, 12 fmuls,
 *          13 fdivs, 14 fsqrts, 15 fmadds, 16 fres, 17 frsqrte
 *          (a multiply-add is FRA x FRC +/- FRB)
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

#define CASES 57
#define CASE_WORDS 11
#define WORDS (3 + CASE_WORDS * CASES)

#define OP_FADD    1
#define OP_FSUB    2
#define OP_FMUL    3
#define OP_FDIV    4
#define OP_FSQRT   5
#define OP_FMADD   6
#define OP_FMSUB   7
#define OP_FNMADD  8
#define OP_FNMSUB  9
#define OP_FADDS   10
#define OP_FSUBS   11
#define OP_FMULS   12
#define OP_FDIVS   13
#define OP_FSQRTS  14
#define OP_FMADDS  15
#define OP_FRES    16
#define OP_FRSQRTE 17

#define ONE     0x3FF0000000000000ull
#define NEGONE  0xBFF0000000000000ull
#define TWO     0x4000000000000000ull
#define INF     0x7FF0000000000000ull
#define NEGINF  0xFFF0000000000000ull
#define NEGZERO 0x8000000000000000ull
#define QNAN    0x7FF8000000000AAAull
#define SNAN    0x7FF0000000000BBBull

/* FPSCR starting values. */
#define CLEAN    0x00000000u
#define FRFIXX   0x02060000u /* XX, FR, FI */
#define ZX_SET   0x04000000u
#define FXZX_SET 0x84000000u
#define VXISI_ON 0x00800000u
#define OX_SET   0x10000000u

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

#define FADD(pre, a, b)      CASE(OP_FADD,   "fadd %4,%1,%2",     pre, a, b, 0)
#define FSUB(pre, a, b)      CASE(OP_FSUB,   "fsub %4,%1,%2",     pre, a, b, 0)
#define FMUL(pre, a, c)      CASE(OP_FMUL,   "fmul %4,%1,%3",     pre, a, 0, c)
#define FDIV(pre, a, b)      CASE(OP_FDIV,   "fdiv %4,%1,%2",     pre, a, b, 0)
#define FSQRT(pre, b)        CASE(OP_FSQRT,  "fsqrt %4,%2",       pre, 0, b, 0)
#define FMADD(pre, a, b, c)  CASE(OP_FMADD,  "fmadd %4,%1,%3,%2", pre, a, b, c)
#define FMSUB(pre, a, b, c)  CASE(OP_FMSUB,  "fmsub %4,%1,%3,%2", pre, a, b, c)
#define FNMADD(pre, a, b, c) CASE(OP_FNMADD, "fnmadd %4,%1,%3,%2", pre, a, b, c)
#define FNMSUB(pre, a, b, c) CASE(OP_FNMSUB, "fnmsub %4,%1,%3,%2", pre, a, b, c)
#define FADDS(pre, a, b)     CASE(OP_FADDS,  "fadds %4,%1,%2",    pre, a, b, 0)
#define FSUBS(pre, a, b)     CASE(OP_FSUBS,  "fsubs %4,%1,%2",    pre, a, b, 0)
#define FMULS(pre, a, c)     CASE(OP_FMULS,  "fmuls %4,%1,%3",    pre, a, 0, c)
#define FDIVS(pre, a, b)     CASE(OP_FDIVS,  "fdivs %4,%1,%2",    pre, a, b, 0)
#define FSQRTS(pre, b)       CASE(OP_FSQRTS, "fsqrts %4,%2",      pre, 0, b, 0)
#define FMADDS(pre, a, b, c) CASE(OP_FMADDS, "fmadds %4,%1,%3,%2", pre, a, b, c)
#define FRES(pre, b)         CASE(OP_FRES,   "fres %4,%2",        pre, 0, b, 0)
#define FRSQRTE(pre, b)      CASE(OP_FRSQRTE, "frsqrte %4,%2",    pre, 0, b, 0)

static void clear_fpscr(void)
{
    double f;
    opnd[0] = 0;
    __asm__ volatile ("lfd %0,0(%1)\n\tmtfsf 0xFF,%0"
                      : "=&f"(f) : "b"(opnd) : "memory");
}

static void cases(void)
{
    /* The multiply-add causes: the product, then the sum with FRB. */
    FMADD(CLEAN, INF, SNAN, 0);          /* 0  inf x 0 + SNaN */
    FMADD(CLEAN, INF, NEGINF, ONE);      /* 1  inf x 1 + -inf */
    FMSUB(CLEAN, INF, INF, ONE);         /* 2  inf x 1 - inf */
    FNMSUB(CLEAN, INF, NEGINF, ONE);     /* 3  -(inf x 1 - -inf) */
    FNMADD(CLEAN, INF, NEGINF, ONE);     /* 4  -(inf x 1 + -inf) */
    FMADD(CLEAN, NEGINF, NEGINF, ONE);   /* 5  -inf x 1 + -inf */
    FMADD(CLEAN, INF, QNAN, 0);          /* 6  inf x 0 + QNaN */
    FMADD(CLEAN, INF, ONE, 0);           /* 7  inf x 0 + 1 */
    FMADD(CLEAN, SNAN, INF, ONE);        /* 8  SNaN x 1 + inf */
    FMADD(CLEAN, INF, INF, NEGINF);      /* 9  inf x -inf + inf */
    FMADDS(CLEAN, INF, ONE, 0);          /* 10 */
    FMADDS(CLEAN, INF, NEGINF, ONE);     /* 11 */

    /* The single-precision arms' causes and zero divide. */
    FADDS(CLEAN, INF, NEGINF);           /* 12 */
    FSUBS(CLEAN, INF, INF);              /* 13 */
    FMULS(CLEAN, 0, INF);                /* 14 */
    FDIVS(CLEAN, TWO, 0);                /* 15 */
    FDIVS(CLEAN, 0, 0);                  /* 16 */
    FDIVS(CLEAN, INF, INF);              /* 17 */
    FSQRTS(CLEAN, NEGONE);               /* 18 */
    FADDS(CLEAN, SNAN, ONE);             /* 19 */
    FDIVS(CLEAN, NEGONE, NEGZERO);       /* 20 */

    /* Double arms: no cause, or one alone. */
    FDIV(CLEAN, INF, 0);                 /* 21 inf / 0: no ZX */
    FDIV(CLEAN, 0, INF);                 /* 22 */
    FDIV(CLEAN, SNAN, 0);                /* 23 SNaN / 0: no ZX */
    FDIV(CLEAN, ONE, NEGZERO);           /* 24 */
    FSUB(CLEAN, NEGINF, NEGINF);         /* 25 */
    FADD(CLEAN, SNAN, QNAN);             /* 26 */
    FMUL(CLEAN, INF, SNAN);              /* 27 */

    /* FR and FI set by an earlier rounding. */
    FSUB(FRFIXX, INF, INF);              /* 28 invalid */
    FDIV(FRFIXX, ONE, 0);                /* 29 zero divide */
    FADD(FRFIXX, ONE, TWO);              /* 30 exact */
    FSQRT(FRFIXX, NEGONE);               /* 31 invalid */
    FMADD(FRFIXX, INF, ONE, 0);          /* 32 invalid */
    FADD(FRFIXX, QNAN, ONE);             /* 33 QNaN operand */
    FADD(FRFIXX, INF, ONE);              /* 34 infinity operand */
    FDIV(FRFIXX, INF, 0);                /* 35 */
    FSQRT(FRFIXX, NEGZERO);              /* 36 */

    /* FX for an exception bit that is already set. */
    FDIV(ZX_SET, ONE, 0);                /* 37 */
    FSUB(VXISI_ON, INF, INF);            /* 38 */
    FDIV(FXZX_SET, ONE, 0);              /* 39 */
    FADD(FXZX_SET, ONE, TWO);            /* 40 */

    /* The estimates on operands that are not powers of two. */
    FRES(CLEAN, 0x3FF8000000000000ull);      /* 41 1.5 */
    FRES(CLEAN, 0x4008000000000000ull);      /* 42 3.0 */
    FRES(CLEAN, 0xBFE6666666666666ull);      /* 43 -0.7 */
    FRES(CLEAN, 0x3FF5555555555555ull);      /* 44 */
    FRSQRTE(CLEAN, 0x3FF8000000000000ull);   /* 45 1.5 */
    FRSQRTE(CLEAN, 0x4008000000000000ull);   /* 46 3.0 */
    FRSQRTE(CLEAN, 0x4001234567890ABCull);   /* 47 */
    FRSQRTE(CLEAN, 0x3FE6666666666666ull);   /* 48 0.7 */
    FRES(FRFIXX, 0x3FF8000000000000ull);     /* 49 */
    FRSQRTE(FRFIXX, 0x4008000000000000ull);  /* 50 */
    FRES(FRFIXX, QNAN);                      /* 51 */
    FRSQRTE(ZX_SET, 0);                      /* 52 */
    FRES(OX_SET, 0x0000000000000001ull);     /* 53 */

    /* Single denormal results the arm computes exactly. */
    FDIVS(CLEAN, 0x3730000000000000ull, ONE);            /* 54 2^-140 */
    FMADDS(CLEAN, 0x3B40000000000000ull, 0,
           0x3B50000000000000ull);                       /* 55 2^-149 */
    FSQRT(CLEAN, 0x4010000000000000ull);                 /* 56 sqrt 4 */
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
