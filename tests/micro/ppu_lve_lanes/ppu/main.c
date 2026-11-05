/* PPU probe: what lvebx, lvehx and lvewx leave in the lanes of vD the
 * load does not name.
 *
 * Line M holds bytes 0x10..0x1F and is the line every element load
 * reads. Line P holds bytes 0xE0..0xEF and presets vD. Each element load
 * runs at every offset 0..15 into line M, in two variants:
 *   register  vD = P through a vor from another register, then the load
 *   fresh     vD = P by an lvx straight before the load
 * Each (instruction, offset, variant) runs twice. The full vD after the
 * load is stored with stvx.
 *
 * Payload (big-endian):
 *   word 0  0x4C564531 ("LVE1") once every case ran   word 1  CASES
 *   words 2..3  0
 *   then 16 bytes per run, at 16 + 16 * (((o * 16 + k) * 2 + v) * 2 + r)
 *   for instruction o (0 lvebx, 1 lvehx, 2 lvewx), offset k, variant v
 *   (0 register, 1 fresh) and run r.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define OPS 3
#define OFFSETS 16
#define VARIANTS 2
#define RUNS 2
#define CASES (OPS * OFFSETS * VARIANTS * RUNS)
#define BYTES (16 + 16 * CASES)
#define DONE 0x4C564531u

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned char result[BYTES] __attribute__((aligned(128)));
static volatile unsigned char line_m[16] __attribute__((aligned(128)));
static volatile unsigned char line_p[16] __attribute__((aligned(128)));

#define LVE_FN(fn, insn)                                                   \
    static void fn(unsigned long off, int fresh, volatile unsigned char *out) \
    {                                                                      \
        if (fresh)                                                         \
            __asm__ volatile ("lvx 1,0,%1\n\t"                             \
                              insn " 1,%0,%2\n\t"                          \
                              "stvx 1,0,%3"                                \
                              : : "b"(line_m), "r"(line_p), "r"(off),      \
                                  "r"(out)                                 \
                              : "v1", "memory");                           \
        else                                                               \
            __asm__ volatile ("lvx 2,0,%1\n\t"                             \
                              "vor 1,2,2\n\t"                              \
                              "vor 3,2,2\n\t"                              \
                              "vor 3,3,3\n\t"                              \
                              "vor 3,3,3\n\t"                              \
                              insn " 1,%0,%2\n\t"                          \
                              "stvx 1,0,%3"                                \
                              : : "b"(line_m), "r"(line_p), "r"(off),      \
                                  "r"(out)                                 \
                              : "v1", "v2", "v3", "memory");               \
    }

LVE_FN(run_lvebx, "lvebx")
LVE_FN(run_lvehx, "lvehx")
LVE_FN(run_lvewx, "lvewx")

static void put_word(unsigned int at, unsigned int x)
{
    result[at] = (x >> 24) & 0xFF;
    result[at + 1] = (x >> 16) & 0xFF;
    result[at + 2] = (x >> 8) & 0xFF;
    result[at + 3] = x & 0xFF;
}

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

int main(void)
{
    unsigned int i, o, k, v, r;

    memset((void *)result, 0, sizeof(result));
    for (i = 0; i < 16; i++) {
        line_m[i] = 0x10 + i;
        line_p[i] = 0xE0 + i;
    }
    put_word(4, CASES);

    for (o = 0; o < OPS; o++)
        for (k = 0; k < OFFSETS; k++)
            for (v = 0; v < VARIANTS; v++)
                for (r = 0; r < RUNS; r++) {
                    volatile unsigned char *out =
                        &result[16 + 16 * (((o * OFFSETS + k) * VARIANTS + v)
                                           * RUNS + r)];
                    if (o == 0)
                        run_lvebx(k, v, out);
                    else if (o == 1)
                        run_lvehx(k, v, out);
                    else
                        run_lvewx(k, v, out);
                }

    put_word(0, DONE);
    write_frame();
    return 0;
}
