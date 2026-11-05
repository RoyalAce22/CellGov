/* PPU probe: fsqrts on the double (2^24 + 1)^2, whose exact root
 * 2^24 + 1 lies halfway between the singles 2^24 and 2^24 + 2.
 *
 * - fsqrts in each rounding mode: how a root that is an exact tie at
 *   single precision rounds;
 * - fsqrt of the same operand: the exact double root.
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
 *     +0   operation: 5 fsqrt, 14 fsqrts
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

#define CASES 5
#define CASE_WORDS 11
#define WORDS (3 + CASE_WORDS * CASES)

#define OP_FSQRT   5
#define OP_FSQRTS  14

#define TIE     0x42F0000020000010ull /* (2^24 + 1)^2 */

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

#define FSQRT(pre, b)     CASE(OP_FSQRT,  "fsqrt %4,%2",    pre, 0, b, 0)
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
        FSQRTS(RN(k), TIE);              /* 0-3 */
    FSQRT(RN(0), TIE);                   /* 4 */
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
