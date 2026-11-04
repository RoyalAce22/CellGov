/* PPU probe: the VRSAVE value a PPU thread sees at entry, and the
 * mfvrsave widening.
 *
 * main reads VRSAVE first, writes 0xC0000001 and reads it back, then
 * creates a child thread whose entry reads VRSAVE before anything else.
 * The parent's VRSAVE is non-zero when the child starts, so the child's
 * word tells a reset value from an inherited one.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code
 *   2  VRSAVE at main
 *   3  low word of RT after mtvrsave 0xC0000001; mfvrsave
 *   4  high word of that RT
 *   5  VRSAVE at the child's entry
 *   6  the child's exit value as join returned it
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
#define WORDS 7

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

static void child_entry(void *arg)
{
    unsigned long long v;
    (void)arg;
    __asm__ volatile ("mfspr %0,256" : "=r"(v));
    result[5] = (unsigned int)v;
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
    unsigned long long v;
    unsigned long long w = 0xFFFFFFFFC0000001ull;
    unsigned long long tid = 0;
    unsigned long long retval = 0;
    s32 ret;

    __asm__ volatile ("mfspr %0,256" : "=r"(v));
    memset((void *)result, 0, sizeof(result));
    result[2] = (unsigned int)v;

    __asm__ volatile ("mtspr 256,%1\n\tmfspr %0,256" : "=r"(v) : "r"(w));
    result[3] = (unsigned int)v;
    result[4] = (unsigned int)(v >> 32);

    result[5] = 0xDEADBEEF;
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
    result[6] = (unsigned int)retval;
    write_frame();
    return 0;
}
