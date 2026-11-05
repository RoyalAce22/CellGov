/* PPU probe: what fdivs and fsqrts write under OE=1 and UE=1 when an
 * operand lies outside the single range.
 *
 * Such an operand makes the operation round at double precision. The
 * probe asks which exponent adjustment an enabled overflow or underflow
 * then applies (1536, the double one, or 192, the single one), and
 * whether a result outside the single range but inside the double range
 * raises OX or UX at all. Beside them: the same enables with every
 * operand in the single range, and the double-precision cases with the
 * enables clear.
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
 *     +0   operation: 13 fdivs, 14 fsqrts
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

#define CASES 16
#define CASE_WORDS 11
#define WORDS (3 + CASE_WORDS * CASES)

#define OP_FDIVS   13
#define OP_FSQRTS  14

#define SENTINEL 0x1234567811223344ull

#define ONE     0x3FF0000000000000ull
#define TWO     0x4000000000000000ull
#define THREE   0x4008000000000000ull
#define HALF    0x3FE0000000000000ull
#define DMAX    0x7FEFFFFFFFFFFFFFull
#define DMIN    0x0010000000000000ull /* 2^-1022 */
#define DDEN    0x0000000000000001ull /* 2^-1074 */
#define SMAX    0x47EFFFFFE0000000ull /* the largest single */
#define P128    0x47F0000000000000ull /* 2^128 */
#define M126    0x3810000000000000ull /* 2^-126 */
#define M130    0x37D0000000000000ull /* 2^-130 */

/* FPSCR starting values: FPRF 0b01101, FR and FI set. */
#define DIRTY   0x0006D000u
#define OE      0x40u
#define UE      0x20u
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
                          : "b"(opnd), "b"(fout) : "memory");                \
        put_case(op);                                                        \
    } while (0)

#define FDIVS(pre, a, b)   CASE(OP_FDIVS,  "fdivs %4,%1,%2", pre, a, b, 0)
#define FSQRTS(pre, b)     CASE(OP_FSQRTS, "fsqrts %4,%2",   pre, 0, b, 0)

static void clear_fpscr(void)
{
    double f;
    opnd[0] = 0;
    __asm__ volatile ("lfd %0,0(%1)\n\tmtfsf 0xFF,%0"
                      : "=&f"(f) : "b"(opnd) : "memory");
}

static void cases(void)
{
    /* 0-6: fdivs with an operand outside the single range. */
    FDIVS(DIRTY | OE, DMAX, HALF);          /* 0 overflows the double range */
    FDIVS(DIRTY | OE | RN(1), DMAX, HALF);  /* 1 the same, toward zero */
    FDIVS(DIRTY | OE, ONE, DDEN);           /* 2 2^1074 */
    FDIVS(DIRTY | OE, P128, M126);          /* 3 2^254: past single, inside double */
    FDIVS(DIRTY | UE, DDEN, TWO);           /* 4 2^-1075, exact */
    FDIVS(DIRTY | UE, DMIN, THREE);         /* 5 tiny and inexact */
    FDIVS(DIRTY | UE, M130, P128);          /* 6 2^-258: below single, inside double */

    /* 7-9: fsqrts with an operand outside the single range. */
    FSQRTS(DIRTY | OE, DMAX);               /* 7 near 2^512 */
    FSQRTS(DIRTY | UE, DDEN);               /* 8 2^-537 */
    FSQRTS(DIRTY | UE, DMIN);               /* 9 2^-511 */

    /* 10-13: fdivs with every operand in the single range. */
    FDIVS(DIRTY | OE, SMAX, HALF);          /* 10 */
    FDIVS(DIRTY | OE, ONE, M130);           /* 11 2^130 */
    FDIVS(DIRTY | UE, M126, THREE);         /* 12 tiny and inexact */
    FDIVS(DIRTY | UE, M126, TWO);           /* 13 2^-127, exact */

    /* 14-15: the enables clear. */
    FDIVS(DIRTY, DMAX, HALF);               /* 14 */
    FDIVS(DIRTY, DDEN, TWO);                /* 15 */
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
