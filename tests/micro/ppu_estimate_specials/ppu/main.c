/* PPU probe: what fres and frsqrte return for each special operand the
 * PowerPC book tabulates, and for a few ordinary operands.
 *
 * Each case loads FRB from memory, runs one instruction under the
 * process's default FPSCR, and stores FRT. Operands:
 * QNAN = 0xFFF8000000001234, SNAN = 0x7FF0000000005678.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code
 *   2 + 2c, case c (high word, low word of FRT):
 *     fres:     0 -INF    1 -0      2 +0      3 +INF    4 SNAN
 *               5 QNAN    6 2.0     7 -0.25   8 1+2^-24+2^-52
 *               9 1.0    10 3.0    11 2^-200
 *     frsqrte: 12 -INF   13 -1.0   14 -denormal (0x8000000000000001)
 *              15 -0     16 +0     17 +INF   18 SNAN   19 QNAN
 *              20 4.0    21 1.0    22 2.0    23 3.0
 *  50  FPSCR low word before the cases   51  after them
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 52

#define INF     0x7FF0000000000000ull
#define NEGINF  0xFFF0000000000000ull
#define NEGZERO 0x8000000000000000ull
#define QNAN    0xFFF8000000001234ull
#define SNAN    0x7FF0000000005678ull

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned long long opnd __attribute__((aligned(8)));
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

static unsigned int fpscr_low(void)
{
    double f;
    __asm__ volatile ("mffs %0\n\tstfd %0,0(%1)" : "=&f"(f) : "b"(&fout) : "memory");
    return (unsigned int)fout;
}

static void put_case(unsigned int c)
{
    result[2 + 2 * c] = (unsigned int)(fout >> 32);
    result[3 + 2 * c] = (unsigned int)fout;
}

/* One estimate instruction: FRB from memory, FRT stored to fout. */
#define EST(c, insn, y)                                                      \
    do {                                                                     \
        double fb, ft;                                                       \
        opnd = (y);                                                          \
        __asm__ volatile ("lfd %1,0(%2)\n\t" insn " %0,%1\n\tstfd %0,0(%3)"  \
                          : "=&f"(ft), "=&f"(fb)                             \
                          : "b"(&opnd), "b"(&fout) : "memory");              \
        put_case(c);                                                         \
    } while (0)

int main(void)
{
    memset((void *)result, 0, sizeof(result));
    result[50] = fpscr_low();

    EST(0, "fres", NEGINF);
    EST(1, "fres", NEGZERO);
    EST(2, "fres", 0ull);
    EST(3, "fres", INF);
    EST(4, "fres", SNAN);
    EST(5, "fres", QNAN);
    EST(6, "fres", 0x4000000000000000ull);
    EST(7, "fres", 0xBFD0000000000000ull);
    EST(8, "fres", 0x3FF0000010000001ull);
    EST(9, "fres", 0x3FF0000000000000ull);
    EST(10, "fres", 0x4008000000000000ull);
    EST(11, "fres", 0x2370000000000000ull);

    EST(12, "frsqrte", NEGINF);
    EST(13, "frsqrte", 0xBFF0000000000000ull);
    EST(14, "frsqrte", 0x8000000000000001ull);
    EST(15, "frsqrte", NEGZERO);
    EST(16, "frsqrte", 0ull);
    EST(17, "frsqrte", INF);
    EST(18, "frsqrte", SNAN);
    EST(19, "frsqrte", QNAN);
    EST(20, "frsqrte", 0x4010000000000000ull);
    EST(21, "frsqrte", 0x3FF0000000000000ull);
    EST(22, "frsqrte", 0x4000000000000000ull);
    EST(23, "frsqrte", 0x4008000000000000ull);

    result[51] = fpscr_low();
    write_frame();
    return 0;
}
