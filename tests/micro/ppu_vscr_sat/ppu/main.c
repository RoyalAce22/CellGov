/* PPU probe: the VSCR register itself -- mfvscr and mtvscr widths, the
 * reserved bits, and when SAT is set.
 *
 * Each step writes VSCR with mtvscr, runs at most one vector
 * instruction, and records the low word of mfvscr. The vector
 * instructions splat their operands across all lanes unless a step says
 * otherwise. VSCR is restored to its entry value before the frame is
 * written.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code
 *   2..5  the full mfvscr vector after mtvscr of
 *         { FFFFFFFF, FFFFFFFF, FFFFFFFF, 00010001 }, with the target
 *         register all ones before mfvscr
 *   6  VSCR after mtvscr 0xFFFFFFFF
 *   7  VSCR after mtvscr 0xA5A5A5A5
 *   8  VSCR after mtvscr 0; vsububs(5, 5)        (result at the bound)
 *   9  word 0 of that vsububs result
 *  10  VSCR after vsububs(a, 5), a = 3 in byte 0 and 9 elsewhere
 *  11  VSCR after a further vsububs(5, 5)          (no clamp)
 *  12  VSCR after mtvscr 0
 *  13  VSCR after vaddubm(0xFF, 0x01)              (modulo wrap)
 *  14  word 0 of that vaddubm result
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 15

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));

static volatile unsigned int va[4] __attribute__((aligned(16)));
static volatile unsigned int vb[4] __attribute__((aligned(16)));
static volatile unsigned int vd[4] __attribute__((aligned(16)));
static volatile unsigned int vs[4] __attribute__((aligned(16)));

static void splat(volatile unsigned int *v, unsigned int x)
{
    v[0] = x; v[1] = x; v[2] = x; v[3] = x;
}

static unsigned int read_vscr(void)
{
    __asm__ volatile ("mfvscr 0\n\tstvx 0,0,%0"
                      : : "r"(vs) : "v0", "memory");
    return vs[3];
}

/* mtvscr of the vector in vs, all four words as the caller left them. */
static void write_vscr_vector(void)
{
    __asm__ volatile ("lvx 0,0,%0\n\tmtvscr 0"
                      : : "r"(vs) : "v0", "memory");
}

static void write_vscr(unsigned int x)
{
    splat(vs, x);
    write_vscr_vector();
}

static unsigned int vsububs(void)
{
    __asm__ volatile ("lvx 1,0,%0\n\tlvx 2,0,%1\n\t"
                      "vsububs 3,1,2\n\tstvx 3,0,%2"
                      : : "r"(va), "r"(vb), "r"(vd)
                      : "v1", "v2", "v3", "memory");
    return vd[0];
}

static unsigned int vaddubm(void)
{
    __asm__ volatile ("lvx 1,0,%0\n\tlvx 2,0,%1\n\t"
                      "vaddubm 3,1,2\n\tstvx 3,0,%2"
                      : : "r"(va), "r"(vb), "r"(vd)
                      : "v1", "v2", "v3", "memory");
    return vd[0];
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
    unsigned int entry_vscr = read_vscr();

    memset((void *)result, 0, sizeof(result));

    /* mtvscr reads only word 3; mfvscr zeroes words 0..2. */
    vs[0] = 0xFFFFFFFFu; vs[1] = 0xFFFFFFFFu; vs[2] = 0xFFFFFFFFu;
    vs[3] = 0x00010001u;
    write_vscr_vector();
    splat(vd, 0xFFFFFFFFu);
    __asm__ volatile ("lvx 4,0,%0\n\tmfvscr 4\n\tstvx 4,0,%0"
                      : : "r"(vd) : "v4", "memory");
    result[2] = vd[0];
    result[3] = vd[1];
    result[4] = vd[2];
    result[5] = vd[3];

    write_vscr(0xFFFFFFFFu);
    result[6] = read_vscr();
    write_vscr(0xA5A5A5A5u);
    result[7] = read_vscr();

    write_vscr(0);
    splat(va, 0x05050505u);
    splat(vb, 0x05050505u);
    result[9] = vsububs();
    result[8] = read_vscr();

    splat(va, 0x09090909u);
    va[0] = 0x03090909u;
    vsububs();
    result[10] = read_vscr();

    splat(va, 0x05050505u);
    vsububs();
    result[11] = read_vscr();

    write_vscr(0);
    result[12] = read_vscr();

    splat(va, 0xFFFFFFFFu);
    splat(vb, 0x01010101u);
    result[14] = vaddubm();
    result[13] = read_vscr();

    write_vscr(entry_vscr);
    write_frame();
    return 0;
}
