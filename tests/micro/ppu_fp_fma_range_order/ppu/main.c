/* PPU probe: the single multiply-add forms on an operand that single
 * format cannot hold.
 *
 * - FR and FI set before an operand out of the single range;
 * - an infinite FRB beside an out-of-range FRA or FRC;
 * - an infinite FRC beside an out-of-range FRA;
 * - infinity times zero, infinity minus infinity and an SNaN beside an
 *   out-of-range operand;
 * - the sign of the result of fmsubs, fnmadds and fnmsubs on an
 *   out-of-range operand.
 *
 * Each case sets the whole FPSCR to its starting value with mtfsf 0xFF
 * (every enable bit 0 in every case; RN 0), loads its operands from
 * memory, runs one instruction, stores the result and then the mffs
 * result. The probe clears the FPSCR before it exits. No system call
 * other than the TTY and result-file writes.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code   2  case count
 *   3 + 11k, case k:
 *     +0   operation: 15 fmadds, 18 fmsubs, 19 fnmadds, 20 fnmsubs
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

#define CASES 18
#define CASE_WORDS 11
#define WORDS (3 + CASE_WORDS * CASES)

#define OP_FMADDS  15
#define OP_FMSUBS  18
#define OP_FNMADDS 19
#define OP_FNMSUBS 20

#define ONE     0x3FF0000000000000ull
#define INF     0x7FF0000000000000ull
#define NEGINF  0xFFF0000000000000ull
#define SNAN    0x7FF0000000000BBBull

#define P128    0x47F0000000000000ull /* 2^128 */
#define M150    0x3690000000000000ull /* 2^-150 */
#define P200    0x4C70000000000000ull /* 2^200 */
#define M200    0x3370000000000000ull /* 2^-200 */

/* FPSCR starting values. */
#define CLEAN    0x00000000u
#define FRFIXX   0x02060000u /* XX, FR, FI */

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

static void cases(void)
{
    /* FR and FI set before an out-of-range operand, and two controls. */
    FMADDS(FRFIXX, P200, 0, ONE);        /* 0 */
    FMADDS(FRFIXX, ONE, P128, ONE);      /* 1 */
    FMADDS(FRFIXX, ONE, 0, M150);        /* 2 */
    FMADDS(FRFIXX, P200, INF, ONE);      /* 3 */
    FMADDS(FRFIXX, ONE, ONE, ONE);       /* 4 */

    /* An infinite FRB or FRC beside an out-of-range operand. */
    FMADDS(CLEAN, P200, INF, ONE);       /* 5 */
    FMADDS(CLEAN, ONE, NEGINF, P200);    /* 6 */
    FMADDS(CLEAN, M200, INF, ONE);       /* 7 */
    FMADDS(CLEAN, P200, ONE, INF);       /* 8 */

    /* An invalid operation or an SNaN beside an out-of-range operand. */
    FMADDS(CLEAN, INF, P200, 0);         /* 9  infinity times zero */
    FMADDS(CLEAN, INF, NEGINF, P200);    /* 10 infinity minus infinity */
    FMADDS(CLEAN, SNAN, P200, ONE);      /* 11 */
    FMADDS(CLEAN, ONE, SNAN, P200);      /* 12 */

    /* The other single forms on an out-of-range operand. */
    FMSUBS(CLEAN, P200, 0, ONE);         /* 13 */
    FNMADDS(CLEAN, P200, 0, ONE);        /* 14 */
    FNMSUBS(CLEAN, ONE, P128, ONE);      /* 15 */
    FMSUBS(CLEAN, P200, INF, ONE);       /* 16 */
    FNMADDS(CLEAN, P200, INF, ONE);      /* 17 */
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
