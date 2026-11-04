/* PPU probe: the vector float mode a PPU thread runs under.
 *
 * main reads VSCR first, then writes NJ = 0 and NJ = 1 with mtvscr and
 * reads each back. Under each NJ value it runs five operations whose
 * results tell round to nearest from round toward zero, IEEE Inf/NaN
 * from the graphics rounding mode, and denormal results from flushing.
 * Last, main writes NJ = 0, the opposite of the entry value, and creates
 * a child thread. The child reads VSCR at entry, writes NJ = 1 and reads
 * it back; main reads its own VSCR again after the join. The child's
 * entry word tells a reset value from an inherited one, and main's last
 * word tells per-thread NJ from a shared one.
 *
 * Each operation splats its operands across all four lanes and records
 * lane 0. A VSCR word is the low word of the mfvscr vector: NJ is
 * 0x00010000, SAT is 0x00000001.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code
 *   2  VSCR at main entry
 *   3  VSCR after mtvscr 0
 *   4  VSCR after mtvscr 0x00010000
 *   5..9   under NJ = 0:
 *          5  vaddfp(1.0, 0x33C00000)
 *          6  vaddfp(+Inf, -Inf)
 *          7  vmaddfp(0x7F7FFFFF, 2.0, -0.0)
 *          8  vaddfp(0x00000001, +0.0)
 *          9  vcfsx(0x7FFFFFFF, 0)
 *   10..14 the same five under NJ = 1
 *   15 VSCR at the child's entry (main holds NJ = 0)
 *   16 the child's VSCR after it writes 0x00010000
 *   17 main's VSCR after the join
 *   18 the child's exit value as join returned it
 */

#include <string.h>

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PPU_THREAD_EXIT   41
#define SYS_PPU_THREAD_JOIN   44
#define WORDS 19

#define VSCR_NJ 0x00010000u

/* The thread-init block and OPD repacking follow
 * ppu_two_threads_disjoint_writes. */
struct elfv1_opd {
    unsigned long long code;
    unsigned long long toc;
    unsigned long long env;
};

struct cg_thread_param {
    unsigned int entry_opd_ptr;
    unsigned int tls;
    unsigned int opd_code;
    unsigned int opd_toc;
};

static unsigned long make_thread_param(struct cg_thread_param *p, const void *fn)
{
    const struct elfv1_opd *desc = (const struct elfv1_opd *)fn;
    unsigned long tls_reg;
    __asm__ volatile ("mr %0, 13" : "=r"(tls_reg));
    p->opd_code = (unsigned int)desc->code;
    p->opd_toc = (unsigned int)desc->toc;
    p->entry_opd_ptr = (unsigned int)(unsigned long)&p->opd_code;
    p->tls = (unsigned int)tls_reg;
    return (unsigned long)p;
}

static struct cg_thread_param child_param __attribute__((aligned(8)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));

/* Operand and result vectors. The child has its own VSCR buffer so the
 * two threads never share one. */
static volatile unsigned int va[4] __attribute__((aligned(16)));
static volatile unsigned int vb[4] __attribute__((aligned(16)));
static volatile unsigned int vc[4] __attribute__((aligned(16)));
static volatile unsigned int vd[4] __attribute__((aligned(16)));
static volatile unsigned int vscr_main[4] __attribute__((aligned(16)));
static volatile unsigned int vscr_child[4] __attribute__((aligned(16)));

static void splat(volatile unsigned int *v, unsigned int x)
{
    v[0] = x; v[1] = x; v[2] = x; v[3] = x;
}

static unsigned int read_vscr(volatile unsigned int *buf)
{
    __asm__ volatile ("mfvscr 0\n\tstvx 0,0,%0"
                      : : "r"(buf) : "v0", "memory");
    return buf[3];
}

static void write_vscr(volatile unsigned int *buf, unsigned int x)
{
    splat(buf, x);
    __asm__ volatile ("lvx 0,0,%0\n\tmtvscr 0"
                      : : "r"(buf) : "v0", "memory");
}

static unsigned int vaddfp(unsigned int a, unsigned int b)
{
    splat(va, a);
    splat(vb, b);
    __asm__ volatile ("lvx 1,0,%0\n\tlvx 2,0,%1\n\t"
                      "vaddfp 3,1,2\n\tstvx 3,0,%2"
                      : : "r"(va), "r"(vb), "r"(vd)
                      : "v1", "v2", "v3", "memory");
    return vd[0];
}

/* A * C + B, in vmaddfp's vD,vA,vC,vB operand order. */
static unsigned int vmaddfp(unsigned int a, unsigned int c, unsigned int b)
{
    splat(va, a);
    splat(vc, c);
    splat(vb, b);
    __asm__ volatile ("lvx 1,0,%0\n\tlvx 2,0,%1\n\tlvx 4,0,%2\n\t"
                      "vmaddfp 3,1,2,4\n\tstvx 3,0,%3"
                      : : "r"(va), "r"(vc), "r"(vb), "r"(vd)
                      : "v1", "v2", "v3", "v4", "memory");
    return vd[0];
}

static unsigned int vcfsx0(unsigned int b)
{
    splat(vb, b);
    __asm__ volatile ("lvx 2,0,%0\n\tvcfsx 3,2,0\n\tstvx 3,0,%1"
                      : : "r"(vb), "r"(vd)
                      : "v2", "v3", "memory");
    return vd[0];
}

static void run_set(unsigned int base)
{
    result[base + 0] = vaddfp(0x3F800000u, 0x33C00000u);
    result[base + 1] = vaddfp(0x7F800000u, 0xFF800000u);
    result[base + 2] = vmaddfp(0x7F7FFFFFu, 0x40000000u, 0x80000000u);
    result[base + 3] = vaddfp(0x00000001u, 0x00000000u);
    result[base + 4] = vcfsx0(0x7FFFFFFFu);
}

static void child_entry(void *arg)
{
    (void)arg;
    result[15] = read_vscr(vscr_child);
    write_vscr(vscr_child, VSCR_NJ);
    result[16] = read_vscr(vscr_child);
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5AFE);
    for (;;) { }
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
    unsigned int entry_vscr = read_vscr(vscr_main);
    unsigned long long tid = 0;
    unsigned long long retval = 0;
    s32 ret;

    memset((void *)result, 0, sizeof(result));
    result[2] = entry_vscr;

    write_vscr(vscr_main, 0);
    result[3] = read_vscr(vscr_main);
    run_set(5);

    write_vscr(vscr_main, VSCR_NJ);
    result[4] = read_vscr(vscr_main);
    run_set(10);

    write_vscr(vscr_main, 0);
    result[15] = 0xDEADBEEF;
    result[16] = 0xDEADBEEF;
    ret = cgov_ppu_thread_create(
        &tid,
        make_thread_param(&child_param, (const void *)&child_entry),
        0, 0, 1000, 0x4000, THREAD_JOINABLE, 0);
    if (ret != 0) {
        result[0] = 1;
        result[1] = (unsigned int)ret;
        write_frame();
        return 1;
    }
    ret = syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval);
    if (ret != 0) {
        result[0] = 2;
        result[1] = (unsigned int)ret;
        write_frame();
        return 2;
    }
    result[17] = read_vscr(vscr_main);
    result[18] = (unsigned int)retval;
    write_frame();
    return 0;
}
