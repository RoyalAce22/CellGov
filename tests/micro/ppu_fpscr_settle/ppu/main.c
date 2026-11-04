/* PPU probe: whether an FPSCR bit a move instruction writes reads back
 * at once or only after a delay, and the high word mffs writes when the
 * FPSCR is not 0.
 *
 * "Delay" is sync and a 100000-iteration integer loop between the write
 * and the read. No step sets an exception enable bit (FPSCR bits 24:28).
 * The probe clears the FPSCR with mtfsf 0xFF before it exits.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code
 *   2, 3   FPSCR after a clear and mtfsb1 23: at once, after the delay
 *   4, 5   FPSCR after a clear and mtfsfi 3,15: at once, after the delay
 *   6, 7   FPSCR after a clear and mtfsb1 13: at once, after the delay
 *   8, 9   CR after a clear, mtfsb1 23, CR cleared and mcrfs 5,0:
 *          mfcr at once, mfcr after the delay
 *   10     CR after a clear, mtfsb1 23, CR cleared and mcrfs 0,0
 *   11, 12 FPSCR after a clear and fdiv 1/0: at once, after the delay
 *   13, 14 FPSCR after a clear and mtfsf 0xFF from 0x0000F000: at once,
 *          after the delay
 *   15, 16 mffs (high, low) into an FRT holding 0x1234567811223344, after
 *          a clear, mtfsfi 0,8 and mtfsfi 7,3
 *   17, 18 the same with the delay before mffs
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 19

#define ALL_CR "cr0", "cr1", "cr2", "cr3", "cr4", "cr5", "cr6", "cr7"

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned long long opnd __attribute__((aligned(8)));
static volatile unsigned long long opnd2 __attribute__((aligned(8)));
static volatile unsigned long long fout __attribute__((aligned(8)));
static volatile unsigned int spin;

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

static void delay(void)
{
    unsigned int i;
    __asm__ volatile ("sync" ::: "memory");
    for (i = 0; i < 100000; i++)
        spin = spin + i;
    __asm__ volatile ("sync" ::: "memory");
}

static unsigned long long mffs_over(unsigned long long pre)
{
    double f;
    opnd = pre;
    __asm__ volatile ("lfd %0,0(%1)\n\tmffs %0\n\tstfd %0,0(%2)"
                      : "=&f"(f) : "b"(&opnd), "b"(&fout) : "memory");
    return fout;
}

static unsigned int fpscr(void)
{
    return (unsigned int)mffs_over(0);
}

#define MTFSF(flm, v)                                                        \
    do {                                                                     \
        double f;                                                            \
        opnd = (v);                                                          \
        __asm__ volatile ("lfd %0,0(%1)\n\tmtfsf " #flm ",%0"                \
                          : "=&f"(f) : "b"(&opnd) : "memory");               \
    } while (0)

#define CLEAR() MTFSF(0xFF, 0ull)

#define FPSCR_OP(text) __asm__ volatile (text ::: "memory", ALL_CR)

static unsigned int mfcr(void)
{
    unsigned int cr;
    __asm__ volatile ("mfcr %0" : "=r"(cr) :: "memory");
    return cr;
}

static void zero_divide(void)
{
    double fa, fb, ft;
    opnd = 0x3FF0000000000000ull;
    opnd2 = 0;
    __asm__ volatile ("lfd %0,0(%3)\n\tlfd %1,0(%4)\n\tfdiv %2,%0,%1"
                      : "=&f"(fa), "=&f"(fb), "=&f"(ft)
                      : "b"(&opnd), "b"(&opnd2) : "memory");
}

int main(void)
{
    unsigned long long v;
    unsigned int cr;

    memset((void *)result, 0, sizeof(result));

    CLEAR();
    FPSCR_OP("mtfsb1 23");
    result[2] = fpscr();
    delay();
    result[3] = fpscr();

    CLEAR();
    FPSCR_OP("mtfsfi 3,15");
    result[4] = fpscr();
    delay();
    result[5] = fpscr();

    CLEAR();
    FPSCR_OP("mtfsb1 13");
    result[6] = fpscr();
    delay();
    result[7] = fpscr();

    CLEAR();
    FPSCR_OP("mtfsb1 23");
    __asm__ volatile ("mtcrf 0xFF,%1\n\tmcrfs 5,0\n\tmfcr %0"
                      : "=&r"(cr) : "r"(0) : "memory", ALL_CR);
    result[8] = cr;
    delay();
    result[9] = mfcr();

    CLEAR();
    FPSCR_OP("mtfsb1 23");
    __asm__ volatile ("mtcrf 0xFF,%1\n\tmcrfs 0,0\n\tmfcr %0"
                      : "=&r"(cr) : "r"(0) : "memory", ALL_CR);
    result[10] = cr;

    CLEAR();
    zero_divide();
    result[11] = fpscr();
    delay();
    result[12] = fpscr();

    CLEAR();
    MTFSF(0xFF, 0x000000000000F000ull);
    result[13] = fpscr();
    delay();
    result[14] = fpscr();

    CLEAR();
    FPSCR_OP("mtfsfi 0,8");
    FPSCR_OP("mtfsfi 7,3");
    v = mffs_over(0x1234567811223344ull);
    result[15] = (unsigned int)(v >> 32);
    result[16] = (unsigned int)v;
    delay();
    v = mffs_over(0x1234567811223344ull);
    result[17] = (unsigned int)(v >> 32);
    result[18] = (unsigned int)v;

    CLEAR();
    write_frame();
    return 0;
}
