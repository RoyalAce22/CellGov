/* PPU probe: which FPSCR bits the move instructions can change. An
 * arithmetic instruction sets status bits (fdiv 1.0 / 0.0 sets ZX and FX
 * under the default enables), then each move instruction tries to clear
 * or set status and control bits.
 *
 * No step sets an exception enable bit (FPSCR bits 24:28). The probe
 * clears the FPSCR with mtfsf 0xFF before it exits.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code
 *   2      FPSCR after fdiv 1/0
 *   3      FPSCR after that and mtfsb0 5 (ZX)
 *   4      FPSCR after that and mtfsb0 0 (FX)
 *   5      FPSCR after fdiv 1/0 and mtfsf 0xFF from 0
 *   6      FPSCR after fdiv 1/0 and mtfsfi 1,0
 *   7, 8   CR, FPSCR after fdiv 1/0 and mcrfs 5,1 (CR cleared first)
 *   9      FPSCR after fdiv 1/0 and mtfsfi 0,0
 *   10     FPSCR after mtfsf 0xFF from 0 and mtfsb1 31 (RN bit 31)
 *   11     FPSCR after mtfsf 0xFF from 0 and mtfsb1 21 (VXSOFT)
 *   12     FPSCR after mtfsf 0xFF from 0 and mtfsfi 0,8 (FX)
 *   13     FPSCR after mtfsf 0xFF from 0 and mtfsfi 4,15 (FPRF bits 16:19)
 *   14     FPSCR after mtfsf 0xFF from 0x00000000_0000F003
 *   15     FPSCR after mtfsf 0xFF from 0x00000000_80000000
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 16

#define ALL_CR "cr0", "cr1", "cr2", "cr3", "cr4", "cr5", "cr6", "cr7"

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

#define FPSCR_OP(text) __asm__ volatile (text ::: "memory", ALL_CR)

#define CR_AFTER(out, text)                                                  \
    __asm__ volatile ("mtcrf 0xFF,%1\n\t" text "\n\tmfcr %0"                 \
                      : "=&r"(out) : "r"(0) : "memory", ALL_CR)

/* fdiv 1.0 / 0.0 from a cleared FPSCR. */
static void zero_divide(void)
{
    double fa, fb, ft;
    CLEAR();
    opnd = 0x3FF0000000000000ull;
    opnd2 = 0;
    __asm__ volatile ("lfd %0,0(%3)\n\tlfd %1,0(%4)\n\tfdiv %2,%0,%1"
                      : "=&f"(fa), "=&f"(fb), "=&f"(ft)
                      : "b"(&opnd), "b"(&opnd2) : "memory");
}

int main(void)
{
    unsigned int cr;

    memset((void *)result, 0, sizeof(result));

    zero_divide();
    result[2] = fpscr();
    FPSCR_OP("mtfsb0 5");
    result[3] = fpscr();
    FPSCR_OP("mtfsb0 0");
    result[4] = fpscr();

    zero_divide();
    CLEAR();
    result[5] = fpscr();

    zero_divide();
    FPSCR_OP("mtfsfi 1,0");
    result[6] = fpscr();

    zero_divide();
    CR_AFTER(cr, "mcrfs 5,1");
    result[7] = cr;
    result[8] = fpscr();

    zero_divide();
    FPSCR_OP("mtfsfi 0,0");
    result[9] = fpscr();

    CLEAR();
    FPSCR_OP("mtfsb1 31");
    result[10] = fpscr();

    CLEAR();
    FPSCR_OP("mtfsb1 21");
    result[11] = fpscr();

    CLEAR();
    FPSCR_OP("mtfsfi 0,8");
    result[12] = fpscr();

    CLEAR();
    FPSCR_OP("mtfsfi 4,15");
    result[13] = fpscr();

    MTFSF(0xFF, 0x000000000000F003ull);
    result[14] = fpscr();

    MTFSF(0xFF, 0x0000000080000000ull);
    result[15] = fpscr();

    CLEAR();
    write_frame();
    return 0;
}
