/* PPU probe: the conversions fctiw, fctiwz, fctid, fctidz, fcfid and
 * frsp under each FPSCR rounding mode.
 *
 * - the rounding of halves and of values below 1 in each mode;
 * - values at and past each end of the 32-bit and 64-bit integer
 *   ranges, before and after rounding;
 * - zeros, denormals, infinities, QNaNs and SNaNs;
 * - the whole FRT, both words, after a convert to integer;
 * - what FPRF, FR and FI hold after each conversion, starting from an
 *   FPSCR whose FPRF, FR and FI are set;
 * - fcfid on integers that need rounding;
 * - frsp on ties, overflow, tiny values and NaNs with payload bits in
 *   the low 29 fraction bits.
 *
 * Each case sets the whole FPSCR to its starting value with mtfsf 0xFF
 * (every enable bit 0 in every case; RN as the case names), loads its
 * operand into FRB, runs one instruction, stores the result and then
 * the mffs result. The probe clears the FPSCR before it exits. No system
 * call other than the TTY and result-file writes.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code   2  case count
 *   3 + 11k, case k:
 *     +0   operation: 1 fctiw, 2 fctiwz, 3 fctid, 4 fctidz, 5 fcfid,
 *          6 frsp (each reads FRB only)
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

#define CASES 153
#define CASE_WORDS 11
#define WORDS (3 + CASE_WORDS * CASES)

#define OP_FCTIW  1
#define OP_FCTIWZ 2
#define OP_FCTID  3
#define OP_FCTIDZ 4
#define OP_FCFID  5
#define OP_FRSP   6

#define SIGN    0x8000000000000000ull
#define INF     0x7FF0000000000000ull
#define NEGINF  0xFFF0000000000000ull
#define NEGZERO 0x8000000000000000ull
#define QNAN    0x7FF8000012345678ull /* payload bits above and in the low 29 */
#define SNAN    0x7FF4000012345678ull
#define DDEN    0x0000000000000001ull /* 2^-1074 */
#define DMAX    0x7FEFFFFFFFFFFFFFull

/* FPSCR starting values: FPRF 0b01101, FR and FI set, RN = k. */
#define CLEAN    0x00000000u
#define DIRTY(k) (0x0006D000u | (unsigned int)(k))

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

/* Runs `insn` from FPSCR `pre` with FRB = %2 and FRT = %4 (FRA and FRC
 * hold 0), then stores FRT and the FPSCR. */
#define CASE(op, insn, pre, b)                                               \
    do {                                                                     \
        double fm, fa, fb, fc, ft, fs;                                       \
        opnd[0] = (pre); opnd[1] = 0; opnd[2] = (b); opnd[3] = 0;            \
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

#define FCTIW(pre, b)  CASE(OP_FCTIW,  "fctiw %4,%2",  pre, b)
#define FCTIWZ(pre, b) CASE(OP_FCTIWZ, "fctiwz %4,%2", pre, b)
#define FCTID(pre, b)  CASE(OP_FCTID,  "fctid %4,%2",  pre, b)
#define FCTIDZ(pre, b) CASE(OP_FCTIDZ, "fctidz %4,%2", pre, b)
#define FCFID(pre, b)  CASE(OP_FCFID,  "fcfid %4,%2",  pre, b)
#define FRSP(pre, b)   CASE(OP_FRSP,   "frsp %4,%2",   pre, b)

static void clear_fpscr(void)
{
    double f;
    opnd[0] = 0;
    __asm__ volatile ("lfd %0,0(%1)\n\tmtfsf 0xFF,%0"
                      : "=&f"(f) : "b"(opnd) : "memory");
}

/* In-range operands every convert to integer rounds. */
static const unsigned long long small[7] = {
    0x4004000000000000ull, /* 2.5 */
    0xC004000000000000ull, /* -2.5 */
    0x400C000000000000ull, /* 3.5 */
    0x3FEFFFFFFFFFFFFFull, /* the double below 1 */
    NEGZERO,
    DDEN,
    0x8000000000000001ull, /* -2^-1074 */
};

/* Operands that are invalid for every convert to integer. */
static const unsigned long long special[5] = {
    INF, NEGINF, QNAN, QNAN | SIGN, SNAN,
};

static void cases(void)
{
    unsigned int k, i;

    /* 0-55: fctiw and fctid on each small operand in each mode. */
    for (k = 0; k < 4; k++)
        for (i = 0; i < 7; i++) {
            FCTIW(DIRTY(k), small[i]);
            FCTID(DIRTY(k), small[i]);
        }
    /* 56-69: the toward-zero forms. */
    for (i = 0; i < 7; i++) {
        FCTIWZ(DIRTY(0), small[i]);
        FCTIDZ(DIRTY(0), small[i]);
    }

    /* 70-79: the ends of the 32-bit range. 2^31 - 0.5 rounds up to 2^31
     * in RN 0 and down in RN 1; -2^31 - 0.5 rounds to -2^31 in RN 0 and
     * past it in RN 3. */
    FCTIW(DIRTY(0), 0x41DFFFFFFFE00000ull);  /* 70 2^31 - 0.5 */
    FCTIW(DIRTY(0), 0xC1E0000000100000ull);  /* 71 -2^31 - 0.5 */
    FCTIW(DIRTY(0), 0x41E0000000000000ull);  /* 72 2^31 */
    FCTIW(DIRTY(0), 0xC1E0000000000000ull);  /* 73 -2^31 */
    FCTIW(DIRTY(0), 0xC1E0000000200000ull);  /* 74 -2^31 - 1 */
    FCTIW(DIRTY(1), 0x41DFFFFFFFE00000ull);  /* 75 */
    FCTIW(DIRTY(3), 0xC1E0000000100000ull);  /* 76 */
    FCTIWZ(DIRTY(0), 0x41DFFFFFFFE00000ull); /* 77 */
    FCTIW(DIRTY(0), DMAX);                   /* 78 */
    /* The ends of the 64-bit range. */
    FCTID(DIRTY(0), 0x43E0000000000000ull);  /* 79 2^63 */
    FCTID(DIRTY(0), 0xC3E0000000000000ull);  /* 80 -2^63 */
    FCTID(DIRTY(0), 0x43DFFFFFFFFFFFFFull);  /* 81 the double below 2^63 */
    FCTID(DIRTY(0), 0xC3E0000000000001ull);  /* 82 below -2^63 */

    /* 83-92: infinities and NaNs. */
    for (i = 0; i < 5; i++) {
        FCTIW(DIRTY(0), special[i]);
        FCTID(DIRTY(0), special[i]);
    }

    /* 93-95: from a clear FPSCR. */
    FCTIW(CLEAN, 0x4004000000000000ull);     /* 93 2.5 */
    FCTID(CLEAN, 0xC01C000000000000ull);     /* 94 -7 */
    FCTIW(CLEAN, QNAN);                      /* 95 */

    /* 96-111: fcfid on integers above 2^53 in each mode. 2^53 + 3 lies
     * halfway between two doubles. */
    for (k = 0; k < 4; k++) {
        FCFID(DIRTY(k), 0x0020000000000001ull);
        FCFID(DIRTY(k), 0x0020000000000003ull);
        FCFID(DIRTY(k), 0xFFDFFFFFFFFFFFFFull); /* -(2^53 + 1) */
        FCFID(DIRTY(k), 0x7FFFFFFFFFFFFFFFull);
    }
    /* 112-115: exact integers. */
    FCFID(DIRTY(0), 0);
    FCFID(DIRTY(0), 0x8000000000000000ull);
    FCFID(DIRTY(0), 0xFFFFFFFFFFFFFFFFull);
    FCFID(DIRTY(0), 1);

    /* 116-143: frsp in each mode. */
    for (k = 0; k < 4; k++) {
        FRSP(DIRTY(k), 0x3FF0000010000000ull); /* 1 + 2^-24, a tie */
        FRSP(DIRTY(k), 0x3FF0000010001000ull); /* just above the tie */
        FRSP(DIRTY(k), 0xBFF0000018000000ull); /* -(1 + 1.5 x 2^-24) */
        FRSP(DIRTY(k), DMAX);
        FRSP(DIRTY(k), 0xC7EFFFFFF0000000ull); /* -(largest single + half ulp) */
        FRSP(DIRTY(k), 0x3690000000000000ull); /* 2^-150 */
        FRSP(DIRTY(k), 0x36A8000000000000ull); /* 1.5 x 2^-149 */
    }
    /* 144-152: exact values, specials and NaN payloads. */
    FRSP(DIRTY(0), NEGZERO);
    FRSP(DIRTY(0), DDEN);
    FRSP(DIRTY(0), INF);
    FRSP(DIRTY(0), QNAN);
    FRSP(DIRTY(0), SNAN);
    FRSP(DIRTY(0), SNAN | SIGN);
    FRSP(DIRTY(0), 0x47F0000000000000ull); /* 2^128 */
    FRSP(DIRTY(0), 0x47EFFFFFE0000000ull); /* the largest single */
    FRSP(DIRTY(0), 0x3810000000000000ull); /* 2^-126 */
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
