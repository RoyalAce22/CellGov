/* PPU probe: what fcmpu and fcmpo write to CR field 6 and to the FPSCR.
 * Each case clears the CR, loads both operands from memory, runs one
 * compare into CR field 6 and reads the CR and the FPSCR back.
 *
 * No step sets an exception enable bit (FPSCR bits 24:28). The probe
 * clears the FPSCR with mtfsf 0xFF before it exits.
 *
 * Payload (big-endian u32 words); each case is a CR word then an FPSCR
 * word, from a cleared FPSCR unless the case says otherwise:
 *   0  failed step (0 = none)   1  its return code
 *   2, 3    fcmpu 1.0, 2.0
 *   4, 5    fcmpu 2.0, 1.0
 *   6, 7    fcmpu +0, -0
 *   8, 9    fcmpu -2.0, -1.0
 *   10, 11  fcmpu -1.0, the smallest positive denormal
 *   12, 13  fcmpu QNaN, 1.0
 *   14, 15  fcmpu 1.0, SNaN
 *   16, 17  fcmpo QNaN, 1.0
 *   18, 19  fcmpo SNaN, 1.0
 *   20, 21  fcmpu 1.0, SNaN again after the previous case and mtfsb0 0
 *           (VXSNAN and VXVC already set, FX cleared)
 *   22, 23  fcmpu 1.0, 2.0 from FPSCR 0x0007F000 (FR, FI, C, FPCC set)
 *   24, 25  fcmpo QNaN, 1.0 from FPSCR 0x00070000 (FR, FI, C set)
 *   26, 27  fcmpo 1.0, 1.0
 *   28, 29  fcmpo -inf, +inf
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 30

#define ALL_CR "cr0", "cr1", "cr2", "cr3", "cr4", "cr5", "cr6", "cr7"

#define ONE     0x3FF0000000000000ull
#define TWO     0x4000000000000000ull
#define NEG_ONE 0xBFF0000000000000ull
#define NEG_TWO 0xC000000000000000ull
#define POS_0   0x0000000000000000ull
#define NEG_0   0x8000000000000000ull
#define DENORM  0x0000000000000001ull
#define QNAN    0x7FF8000000000000ull
#define SNAN    0x7FF0000000000001ull
#define POS_INF 0x7FF0000000000000ull
#define NEG_INF 0xFFF0000000000000ull

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned long long opnd __attribute__((aligned(8)));
static volatile unsigned long long opnd2 __attribute__((aligned(8)));
static volatile unsigned long long fout __attribute__((aligned(8)));

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

static unsigned int fpscr(void)
{
    double f;
    opnd = 0;
    __asm__ volatile ("lfd %0,0(%1)\n\tmffs %0\n\tstfd %0,0(%2)"
                      : "=&f"(f) : "b"(&opnd), "b"(&fout) : "memory");
    return (unsigned int)fout;
}

/* mtfsf FLM from the 64-bit value `v`. FLM must be a literal. */
#define MTFSF(flm, v)                                                        \
    do {                                                                     \
        double f;                                                            \
        opnd = (v);                                                          \
        __asm__ volatile ("lfd %0,0(%1)\n\tmtfsf " #flm ",%0"                \
                          : "=&f"(f) : "b"(&opnd) : "memory");               \
    } while (0)

#define CLEAR() MTFSF(0xFF, 0ull)

/* `insn` 6,FA,FB on `va` and `vb` with the CR cleared first; the CR goes
 * to word `at` and the FPSCR to word `at` + 1. */
#define COMPARE(insn, va, vb, at)                                            \
    do {                                                                     \
        double fa, fb;                                                       \
        unsigned int cr;                                                     \
        opnd = (va);                                                         \
        opnd2 = (vb);                                                        \
        __asm__ volatile ("lfd %1,0(%3)\n\tlfd %2,0(%4)\n\t"                 \
                          "mtcrf 0xFF,%5\n\t"                                \
                          insn " 6,%1,%2\n\tmfcr %0"                         \
                          : "=&r"(cr), "=&f"(fa), "=&f"(fb)                  \
                          : "b"(&opnd), "b"(&opnd2), "r"(0)                  \
                          : "memory", ALL_CR);                               \
        result[(at)] = cr;                                                   \
        result[(at) + 1] = fpscr();                                          \
    } while (0)

int main(void)
{
    memset((void *)result, 0, sizeof(result));

    CLEAR();
    COMPARE("fcmpu", ONE, TWO, 2);
    CLEAR();
    COMPARE("fcmpu", TWO, ONE, 4);
    CLEAR();
    COMPARE("fcmpu", POS_0, NEG_0, 6);
    CLEAR();
    COMPARE("fcmpu", NEG_TWO, NEG_ONE, 8);
    CLEAR();
    COMPARE("fcmpu", NEG_ONE, DENORM, 10);
    CLEAR();
    COMPARE("fcmpu", QNAN, ONE, 12);
    CLEAR();
    COMPARE("fcmpu", ONE, SNAN, 14);
    CLEAR();
    COMPARE("fcmpo", QNAN, ONE, 16);
    CLEAR();
    COMPARE("fcmpo", SNAN, ONE, 18);
    __asm__ volatile ("mtfsb0 0" ::: "memory");
    COMPARE("fcmpu", ONE, SNAN, 20);
    MTFSF(0xFF, 0x000000000007F000ull);
    COMPARE("fcmpu", ONE, TWO, 22);
    MTFSF(0xFF, 0x0000000000070000ull);
    COMPARE("fcmpo", QNAN, ONE, 24);
    CLEAR();
    COMPARE("fcmpo", ONE, ONE, 26);
    CLEAR();
    COMPARE("fcmpo", NEG_INF, POS_INF, 28);

    CLEAR();
    write_frame();
    return 0;
}
