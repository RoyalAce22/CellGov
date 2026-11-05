/* PPU probe: what fres writes under OE=1 when the reciprocal of a tiny
 * operand overflows the single range, and under UE=1 when the
 * reciprocal of a large operand is tiny.
 *
 * The probe asks whether an enabled overflow or underflow scales the
 * estimate's exponent as it scales an arithmetic result, and whether it
 * sets OX, UX, FR, FI and FPRF. Beside each: the same operand with the
 * enables clear, and an operand whose reciprocal stays in range.
 *
 * A PS3 process runs in Ignore Exceptions mode (MSR[FE0:FE1] = 0), so an
 * enabled exception sets FEX and raises no interrupt.
 *
 * Each case sets the whole FPSCR to its starting value with mtfsf 0xFF
 * (FPRF 0b01101, FR and FI set, plus the enable bits and RN the case
 * names), loads FRB and a sentinel into FRT, runs one fres, stores FRT
 * and then the mffs result. The probe clears the FPSCR before it exits.
 * No system call other than the TTY and result-file writes.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code   2  case count
 *   3 + 11k, case k:
 *     +0   operation: 24 fres
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

#define CASES 13
#define CASE_WORDS 11
#define WORDS (3 + CASE_WORDS * CASES)

#define OP_FRES    24

#define SENTINEL 0x1234567811223344ull

#define DMAX    0x7FEFFFFFFFFFFFFFull
#define DMIN    0x0010000000000000ull /* 2^-1022 */
#define DDEN    0x0000000000000001ull /* 2^-1074 */
#define P100    0x4630000000000000ull /* 2^100 */
#define P126    0x47D0000000000000ull /* 2^126 */
#define P127    0x47E0000000000000ull /* 2^127 */
#define P128    0x47F0000000000000ull /* 2^128 */
#define M126    0x3810000000000000ull /* 2^-126 */
#define M130    0x37D0000000000000ull /* 2^-130 */
#define NEGM130 0xB7D0000000000000ull /* -2^-130 */

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

/* Runs fres from FPSCR `pre` with FRB = %2 and FRT = %4 holding the
 * sentinel, then stores FRT and the FPSCR. */
#define FRES(pre, b)                                                         \
    do {                                                                     \
        double fm, fa, fb, fc, ft, fs;                                       \
        opnd[0] = (pre); opnd[1] = 0; opnd[2] = (b); opnd[3] = 0;            \
        opnd[4] = SENTINEL;                                                  \
        __asm__ volatile ("lfd %0,0(%6)\n\tmtfsf 0xFF,%0\n\t"                \
                          "lfd %1,8(%6)\n\tlfd %2,16(%6)\n\t"                \
                          "lfd %3,24(%6)\n\tlfd %4,32(%6)\n\t"               \
                          "fres %4,%2\n\tstfd %4,0(%7)\n\t"                  \
                          "mffs %5\n\tstfd %5,8(%7)"                         \
                          : "=&f"(fm), "=&f"(fa), "=&f"(fb), "=&f"(fc),      \
                            "=&f"(ft), "=&f"(fs)                             \
                          : "b"(opnd), "b"(fout) : "memory");                \
        put_case(OP_FRES);                                                   \
    } while (0)

static void clear_fpscr(void)
{
    double f;
    opnd[0] = 0;
    __asm__ volatile ("lfd %0,0(%1)\n\tmtfsf 0xFF,%0"
                      : "=&f"(f) : "b"(opnd) : "memory");
}

static void cases(void)
{
    /* 0-6: the reciprocal of a tiny operand overflows. */
    FRES(DIRTY, M130);                   /* 0 OE clear */
    FRES(DIRTY | OE, M130);              /* 1 */
    FRES(DIRTY | OE, NEGM130);           /* 2 */
    FRES(DIRTY | OE | RN(1), M130);      /* 3 toward zero */
    FRES(DIRTY | OE, DMIN);              /* 4 */
    FRES(DIRTY | OE, DDEN);              /* 5 */
    FRES(DIRTY | OE, M126);              /* 6 2^126: no overflow */

    /* 7-12: the reciprocal of a large operand is tiny. */
    FRES(DIRTY, P127);                   /* 7 UE clear */
    FRES(DIRTY | UE, P127);              /* 8 near 2^-127 */
    FRES(DIRTY | UE, P126);              /* 9 near 2^-126, the smallest normal */
    FRES(DIRTY | UE, P128);              /* 10 near 2^-128 */
    FRES(DIRTY | UE, DMAX);              /* 11 far below the single range */
    FRES(DIRTY | UE, P100);              /* 12 2^-100: not tiny */
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
