/* PPU probe: the scalar floating-point rules a program reads back from
 * one instruction -- FR and FI after a rounding, UX on a tiny result,
 * a single-precision result in double format, frsp of a NaN, the sign of
 * an exact zero sum in each rounding mode, the VX bit each invalid
 * operation sets, and which NaN operand of fmadd is the result.
 *
 * Each case sets the whole FPSCR to its rounding mode with mtfsf 0xFF
 * (every status and enable bit 0), loads its operands from memory, runs
 * one instruction, stores the result and then the mffs result. No case
 * sets an exception enable bit. The probe clears the FPSCR before it
 * exits. No system call other than the TTY and result-file writes.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code   2  case count
 *   3 + 11k, case k:
 *     +0   operation: 1 fadd, 2 fsub, 3 fmul, 4 fdiv, 5 fsqrt,
 *          6 fmadd (FRT = FRA x FRC + FRB), 7 fadds, 8 fmuls, 9 frsp
 *     +1   rounding mode (FPSCR RN)
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

#define CASES 51
#define CASE_WORDS 11
#define WORDS (3 + CASE_WORDS * CASES)

#define OP_FADD  1
#define OP_FSUB  2
#define OP_FMUL  3
#define OP_FDIV  4
#define OP_FSQRT 5
#define OP_FMADD 6
#define OP_FADDS 7
#define OP_FMULS 8
#define OP_FRSP  9

#define ONE     0x3FF0000000000000ull
#define NEGONE  0xBFF0000000000000ull
#define HALF    0x3FE0000000000000ull
#define INF     0x7FF0000000000000ull
#define NEGINF  0xFFF0000000000000ull
#define NEGZERO 0x8000000000000000ull
#define QNAN_A  0x7FF8000000000AAAull
#define QNAN_B  0xFFF8000000000BBBull
#define QNAN_C  0x7FF8000000000CCCull
#define SNAN_A  0x7FF0000000000AAAull
#define SNAN_B  0xFFF0000000000BBBull
#define SNAN_C  0x7FF0000000000CCCull

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

/* Records case `next_case`: its operation, the mode in opnd[0], its
 * operands and the two doublewords in fout[]. */
static void put_case(unsigned int op, unsigned long long a,
                     unsigned long long b, unsigned long long c)
{
    unsigned int at = 3 + CASE_WORDS * next_case;
    result[at] = op;
    result[at + 1] = (unsigned int)opnd[0];
    put64(at + 2, a);
    put64(at + 4, b);
    put64(at + 6, c);
    put64(at + 8, fout[0]);
    result[at + 10] = (unsigned int)fout[1];
    next_case++;
}

/* FRT = insn FRA, FRB under mode `rn`. */
#define CASE_AB(op, insn, rn, a, b)                                          \
    do {                                                                     \
        double fm, fa, fb, ft, fs;                                           \
        opnd[0] = (rn); opnd[1] = (a); opnd[2] = (b);                        \
        __asm__ volatile ("lfd %0,0(%5)\n\tmtfsf 0xFF,%0\n\t"                \
                          "lfd %1,8(%5)\n\tlfd %2,16(%5)\n\t"                \
                          insn " %3,%1,%2\n\tstfd %3,0(%6)\n\t"              \
                          "mffs %4\n\tstfd %4,8(%6)"                         \
                          : "=&f"(fm), "=&f"(fa), "=&f"(fb), "=&f"(ft),      \
                            "=&f"(fs)                                        \
                          : "b"(opnd), "b"(fout) : "memory");                \
        put_case(op, (a), (b), 0);                                           \
    } while (0)

/* FRT = insn FRA, FRC (the multiplies) under mode `rn`. */
#define CASE_AC(op, insn, rn, a, c)                                          \
    do {                                                                     \
        double fm, fa, fc, ft, fs;                                           \
        opnd[0] = (rn); opnd[1] = (a); opnd[2] = (c);                        \
        __asm__ volatile ("lfd %0,0(%5)\n\tmtfsf 0xFF,%0\n\t"                \
                          "lfd %1,8(%5)\n\tlfd %2,16(%5)\n\t"                \
                          insn " %3,%1,%2\n\tstfd %3,0(%6)\n\t"              \
                          "mffs %4\n\tstfd %4,8(%6)"                         \
                          : "=&f"(fm), "=&f"(fa), "=&f"(fc), "=&f"(ft),      \
                            "=&f"(fs)                                        \
                          : "b"(opnd), "b"(fout) : "memory");                \
        put_case(op, (a), 0, (c));                                           \
    } while (0)

/* FRT = insn FRB under mode `rn`. */
#define CASE_B(op, insn, rn, b)                                              \
    do {                                                                     \
        double fm, fb, ft, fs;                                               \
        opnd[0] = (rn); opnd[1] = (b);                                       \
        __asm__ volatile ("lfd %0,0(%4)\n\tmtfsf 0xFF,%0\n\t"                \
                          "lfd %1,8(%4)\n\t"                                 \
                          insn " %2,%1\n\tstfd %2,0(%5)\n\t"                 \
                          "mffs %3\n\tstfd %3,8(%5)"                         \
                          : "=&f"(fm), "=&f"(fb), "=&f"(ft), "=&f"(fs)       \
                          : "b"(opnd), "b"(fout) : "memory");                \
        put_case(op, 0, (b), 0);                                             \
    } while (0)

/* FRT = FRA x FRC + FRB under mode `rn`. */
#define CASE_MADD(rn, a, b, c)                                               \
    do {                                                                     \
        double fm, fa, fb, fc, ft, fs;                                       \
        opnd[0] = (rn); opnd[1] = (a); opnd[2] = (b); opnd[3] = (c);         \
        __asm__ volatile ("lfd %0,0(%6)\n\tmtfsf 0xFF,%0\n\t"                \
                          "lfd %1,8(%6)\n\tlfd %2,16(%6)\n\t"                \
                          "lfd %3,24(%6)\n\t"                                \
                          "fmadd %4,%1,%3,%2\n\tstfd %4,0(%7)\n\t"           \
                          "mffs %5\n\tstfd %5,8(%7)"                         \
                          : "=&f"(fm), "=&f"(fa), "=&f"(fb), "=&f"(fc),      \
                            "=&f"(ft), "=&f"(fs)                             \
                          : "b"(opnd), "b"(fout) : "memory");                \
        put_case(OP_FMADD, (a), (b), (c));                                   \
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
    unsigned int rn;

    /* FR and FI: 1 + 0.75 ulp, both signs, every mode; and an exact sum. */
    for (rn = 0; rn < 4; rn++) {
        CASE_AB(OP_FADD, "fadd", rn, ONE, 0x3CA8000000000000ull);
        CASE_AB(OP_FADD, "fadd", rn, NEGONE, 0xBCA8000000000000ull);
    }
    CASE_AB(OP_FADD, "fadd", 0, ONE, ONE);
    /* The same in single precision: 1 + 0.75 single ulp. */
    for (rn = 0; rn < 4; rn++)
        CASE_AB(OP_FADDS, "fadds", rn, ONE, 0x3E68000000000000ull);

    /* Tiny results: exact; a tie below the normal range; and a value
     * tiny before rounding that rounds up to the smallest normal. */
    CASE_AC(OP_FMUL, "fmul", 0, 0x0010000000000000ull, HALF);
    CASE_AC(OP_FMUL, "fmul", 0, 0x0010000000000001ull, HALF);
    CASE_AC(OP_FMUL, "fmul", 2, 0x0010000000000001ull, HALF);
    CASE_AC(OP_FMUL, "fmul", 0, 0x001FFFFFFFFFFFFFull, HALF);
    CASE_AC(OP_FMUL, "fmul", 1, 0x001FFFFFFFFFFFFFull, HALF);

    /* Single denormal results, delivered in double format. */
    CASE_AC(OP_FMULS, "fmuls", 0, 0x3B40000000000000ull, 0x3B50000000000000ull);
    CASE_AC(OP_FMULS, "fmuls", 0, 0x3B58000000000000ull, 0x3B40000000000000ull);

    /* frsp of a NaN. */
    CASE_B(OP_FRSP, "frsp", 0, 0x7FF0000123456789ull);
    CASE_B(OP_FRSP, "frsp", 0, 0xFFF800001FFFFFFFull);

    /* The sign of an exact zero sum. */
    for (rn = 0; rn < 4; rn++)
        CASE_AB(OP_FADD, "fadd", rn, ONE, NEGONE);
    CASE_AB(OP_FSUB, "fsub", 3, ONE, ONE);
    CASE_AB(OP_FADD, "fadd", 0, NEGZERO, 0);
    CASE_AB(OP_FADD, "fadd", 3, NEGZERO, 0);
    CASE_AB(OP_FADD, "fadd", 0, NEGZERO, NEGZERO);

    /* One invalid cause per case, and its near misses. */
    CASE_AB(OP_FADD, "fadd", 0, INF, NEGINF);
    CASE_AB(OP_FADD, "fadd", 0, INF, INF);
    CASE_AB(OP_FSUB, "fsub", 0, INF, INF);
    CASE_AC(OP_FMUL, "fmul", 0, 0, INF);
    CASE_AC(OP_FMUL, "fmul", 0, INF, ONE);
    CASE_AB(OP_FDIV, "fdiv", 0, INF, NEGINF);
    CASE_AB(OP_FDIV, "fdiv", 0, NEGZERO, 0);
    CASE_B(OP_FSQRT, "fsqrt", 0, NEGONE);
    CASE_B(OP_FSQRT, "fsqrt", 0, NEGINF);
    CASE_B(OP_FSQRT, "fsqrt", 0, 0x8000000000000001ull);
    CASE_B(OP_FSQRT, "fsqrt", 0, NEGZERO);
    CASE_B(OP_FSQRT, "fsqrt", 0, QNAN_B);
    CASE_AB(OP_FADD, "fadd", 0, SNAN_A, ONE);
    CASE_AB(OP_FADD, "fadd", 0, QNAN_A, ONE);

    /* Which NaN operand of fmadd is the result. */
    CASE_MADD(0, QNAN_A, QNAN_B, QNAN_C);
    CASE_MADD(0, ONE, QNAN_B, QNAN_C);
    CASE_MADD(0, ONE, ONE, SNAN_C);
    CASE_MADD(0, SNAN_A, QNAN_B, ONE);
    CASE_MADD(0, ONE, SNAN_B, QNAN_C);
    CASE_MADD(0, ONE, QNAN_B, SNAN_C);
    CASE_MADD(0, QNAN_A, ONE, SNAN_C);
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
