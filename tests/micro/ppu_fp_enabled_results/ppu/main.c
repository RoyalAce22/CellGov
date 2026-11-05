/* PPU probe: what an FP instruction writes when an FPSCR exception
 * enable bit is set.
 *
 * - VE=1 with an invalid operation and ZE=1 with a zero divide: whether
 *   FRT, FPRF, FR and FI change;
 * - OE=1 with an overflow and UE=1 with an underflow: the scaled result,
 *   in each rounding mode and both widths, with FR, FI, XX and FPRF;
 * - XE=1, and each enable bit with its exception not raised.
 *
 * A PS3 process runs in Ignore Exceptions mode (MSR[FE0:FE1] = 0), so an
 * enabled exception sets FEX and raises no interrupt.
 *
 * Each case sets the whole FPSCR to its starting value with mtfsf 0xFF
 * (FPRF 0b01101, FR and FI set, plus the enable bits and RN the case
 * names), loads FRA, FRB, FRC and a sentinel into FRT, runs one
 * instruction, stores FRT and then the mffs result. The probe clears the
 * FPSCR before it exits. No system call other than the TTY and
 * result-file writes.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code   2  case count
 *   3 + 11k, case k:
 *     +0   operation: 1 fadd, 2 fsub, 3 fmul, 4 fdiv, 5 fsqrt, 6 fmadd,
 *          10 fadds, 12 fmuls, 13 fdivs, 21 frsp, 22 fctiw, 23 fctid,
 *          24 fres, 25 frsqrte, 26 fcmpo (into CR1)
 *     +1   FPSCR low word before the instruction
 *     +2   FRA high, low    +4  FRB high, low    +6  FRC high, low
 *     +8   FRT high, low (FRT holds 0x1234567811223344 before)
 *     +10  FPSCR low word after the instruction
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define CASES 56
#define CASE_WORDS 11
#define WORDS (3 + CASE_WORDS * CASES)

#define OP_FADD    1
#define OP_FSUB    2
#define OP_FMUL    3
#define OP_FDIV    4
#define OP_FSQRT   5
#define OP_FMADD   6
#define OP_FADDS   10
#define OP_FMULS   12
#define OP_FDIVS   13
#define OP_FRSP    21
#define OP_FCTIW   22
#define OP_FCTID   23
#define OP_FRES    24
#define OP_FRSQRTE 25
#define OP_FCMPO   26

#define SENTINEL 0x1234567811223344ull

#define ONE     0x3FF0000000000000ull
#define NEGONE  0xBFF0000000000000ull
#define TWO     0x4000000000000000ull
#define THREE   0x4008000000000000ull
#define HALF    0x3FE0000000000000ull
#define THIRD   0x3FD5555555555555ull /* the double nearest 1/3 */
#define TWOHALF 0x4004000000000000ull /* 2.5 */
#define INF     0x7FF0000000000000ull
#define NEGINF  0xFFF0000000000000ull
#define QNAN    0x7FF8000000000AAAull
#define SNAN    0x7FF0000000000BBBull
#define DMAX    0x7FEFFFFFFFFFFFFFull
#define DMIN    0x0010000000000000ull /* 2^-1022 */
#define DMIN15  0x0018000000000000ull /* 1.5 x 2^-1022 */
#define DDEN    0x0000000000000001ull /* 2^-1074 */
#define SMAX    0x47EFFFFFE0000000ull /* the largest single */
#define P100    0x4630000000000000ull /* 2^100 */
#define P128    0x47F0000000000000ull /* 2^128 */
#define P63     0x43E0000000000000ull /* 2^63 */
#define M60     0x3C30000000000000ull /* 2^-60 */
#define M126    0x3810000000000000ull /* 2^-126 */
#define M130    0x37D0000000000000ull /* 2^-130 */

/* FPSCR starting values: FPRF 0b01101, FR and FI set. */
#define DIRTY   0x0006D000u
#define VE      0x80u
#define OE      0x40u
#define UE      0x20u
#define ZE      0x10u
#define XE      0x08u
#define RN(k)   ((unsigned int)(k))

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned long long opnd[5] __attribute__((aligned(8)));
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
 * FRT = %4 holding the sentinel, then stores FRT and the FPSCR. */
#define CASE(op, insn, pre, a, b, c)                                         \
    do {                                                                     \
        double fm, fa, fb, fc, ft, fs;                                       \
        opnd[0] = (pre); opnd[1] = (a); opnd[2] = (b); opnd[3] = (c);        \
        opnd[4] = SENTINEL;                                                  \
        __asm__ volatile ("lfd %0,0(%6)\n\tmtfsf 0xFF,%0\n\t"                \
                          "lfd %1,8(%6)\n\tlfd %2,16(%6)\n\t"                \
                          "lfd %3,24(%6)\n\tlfd %4,32(%6)\n\t"               \
                          insn "\n\tstfd %4,0(%7)\n\t"                       \
                          "mffs %5\n\tstfd %5,8(%7)"                         \
                          : "=&f"(fm), "=&f"(fa), "=&f"(fb), "=&f"(fc),      \
                            "=&f"(ft), "=&f"(fs)                             \
                          : "b"(opnd), "b"(fout) : "memory", "cr1");         \
        put_case(op);                                                        \
    } while (0)

#define FADD(pre, a, b)    CASE(OP_FADD,  "fadd %4,%1,%2",  pre, a, b, 0)
#define FSUB(pre, a, b)    CASE(OP_FSUB,  "fsub %4,%1,%2",  pre, a, b, 0)
#define FMUL(pre, a, c)    CASE(OP_FMUL,  "fmul %4,%1,%3",  pre, a, 0, c)
#define FDIV(pre, a, b)    CASE(OP_FDIV,  "fdiv %4,%1,%2",  pre, a, b, 0)
#define FSQRT(pre, b)      CASE(OP_FSQRT, "fsqrt %4,%2",    pre, 0, b, 0)
#define FMADD(pre, a, b, c) CASE(OP_FMADD, "fmadd %4,%1,%3,%2", pre, a, b, c)
#define FADDS(pre, a, b)   CASE(OP_FADDS, "fadds %4,%1,%2", pre, a, b, 0)
#define FMULS(pre, a, c)   CASE(OP_FMULS, "fmuls %4,%1,%3", pre, a, 0, c)
#define FDIVS(pre, a, b)   CASE(OP_FDIVS, "fdivs %4,%1,%2", pre, a, b, 0)
#define FRSP(pre, b)       CASE(OP_FRSP,  "frsp %4,%2",     pre, 0, b, 0)
#define FCTIW(pre, b)      CASE(OP_FCTIW, "fctiw %4,%2",    pre, 0, b, 0)
#define FCTID(pre, b)      CASE(OP_FCTID, "fctid %4,%2",    pre, 0, b, 0)
#define FRES(pre, b)       CASE(OP_FRES,  "fres %4,%2",     pre, 0, b, 0)
#define FRSQRTE(pre, b)    CASE(OP_FRSQRTE, "frsqrte %4,%2", pre, 0, b, 0)
#define FCMPO(pre, a, b)   CASE(OP_FCMPO, "fcmpo cr1,%1,%2", pre, a, b, 0)

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

    /* 0-17: VE=1. */
    FADD(DIRTY | VE, INF, NEGINF);       /* 0 VXISI */
    FSUB(DIRTY | VE, SNAN, ONE);         /* 1 VXSNAN */
    FMUL(DIRTY | VE, INF, 0);            /* 2 VXIMZ */
    FDIV(DIRTY | VE, 0, 0);              /* 3 VXZDZ */
    FDIV(DIRTY | VE, INF, INF);          /* 4 VXIDI */
    FSQRT(DIRTY | VE, NEGONE);           /* 5 VXSQRT */
    FMADD(DIRTY | VE, INF, ONE, 0);      /* 6 VXIMZ */
    FADDS(DIRTY | VE, INF, NEGINF);      /* 7 */
    FDIVS(DIRTY | VE, 0, 0);             /* 8 */
    FRSP(DIRTY | VE, SNAN);              /* 9 */
    FCTIW(DIRTY | VE, QNAN);             /* 10 VXCVI */
    FCTID(DIRTY | VE, P63);              /* 11 VXCVI */
    FCTIW(DIRTY | VE, SNAN);             /* 12 VXCVI, VXSNAN */
    FRSQRTE(DIRTY | VE, NEGONE);         /* 13 */
    FRES(DIRTY | VE, SNAN);              /* 14 */
    FCMPO(DIRTY | VE, SNAN, ONE);        /* 15 */
    FCMPO(DIRTY | VE, QNAN, ONE);        /* 16 VXVC */
    FADD(DIRTY | VE, QNAN, ONE);         /* 17 no invalid operation */

    /* 18-23: ZE=1, and the other enable for each. */
    FDIV(DIRTY | ZE, ONE, 0);            /* 18 */
    FDIVS(DIRTY | ZE, ONE, 0);           /* 19 */
    FRES(DIRTY | ZE, 0);                 /* 20 */
    FRSQRTE(DIRTY | ZE, 0);              /* 21 */
    FDIV(DIRTY | ZE, 0, 0);              /* 22 0/0: an invalid operation */
    FDIV(DIRTY | VE, ONE, 0);            /* 23 a zero divide with VE only */

    /* 24-36: OE=1. */
    for (k = 0; k < 4; k++)
        FMUL(DIRTY | OE | RN(k), DMAX, TWO);    /* 24-27 */
    FMUL(DIRTY | OE, DMAX, DMAX);        /* 28 */
    FADD(DIRTY | OE, DMAX, DMAX);        /* 29 */
    FMUL(DIRTY | OE, DMAX, THREE);       /* 30 inexact */
    FMULS(DIRTY | OE, SMAX, TWO);        /* 31 */
    FMULS(DIRTY | OE, P100, P100);       /* 32 */
    FRSP(DIRTY | OE, DMAX);              /* 33 */
    FRSP(DIRTY | OE, P128);              /* 34 */
    FMADD(DIRTY | OE, DMAX, 0, TWO);     /* 35 */
    FDIV(DIRTY | OE, DMAX, HALF);        /* 36 */

    /* 37-48: UE=1. */
    FMUL(DIRTY | UE, DMIN, HALF);        /* 37 exact */
    for (k = 0; k < 4; k++)
        FMUL(DIRTY | UE | RN(k), DMIN, THIRD);  /* 38-41 */
    FMULS(DIRTY | UE, M126, HALF);       /* 42 exact */
    FMULS(DIRTY | UE, M126, THIRD);      /* 43 */
    FRSP(DIRTY | UE, M130);              /* 44 exact */
    FRSP(DIRTY | UE, DDEN);              /* 45 */
    FDIV(DIRTY | UE, DMIN, THREE);       /* 46 */
    FMADD(DIRTY | UE, DMIN, 0, HALF);    /* 47 */
    FSUB(DIRTY | UE, DMIN, DMIN15);      /* 48 exact */

    /* 49-50: XE=1. */
    FADD(DIRTY | XE, ONE, M60);          /* 49 */
    FMUL(DIRTY | XE, DMAX, TWO);         /* 50 overflow with OE=0 */

    /* 51-54: two enables together. */
    FMUL(DIRTY | OE | XE, DMAX, TWO);    /* 51 */
    FMUL(DIRTY | OE | XE, DMAX, THREE);  /* 52 */
    FMUL(DIRTY | UE | XE, DMIN, HALF);   /* 53 */
    FDIV(DIRTY | VE | ZE, 0, 0);         /* 54 */

    /* 55: VE=1 and a valid inexact conversion. */
    FCTIW(DIRTY | VE, TWOHALF);          /* 55 */
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
