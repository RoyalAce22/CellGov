/* PPU program: two PPU threads, each writes to a disjoint memory
 * region, primary joins on the child and reports both values.
 *
 * Structural microtest for multi-PPU threading. It does not
 * exercise any sync primitive or atomic reservation contention.
 * It proves:
 *
 *   1. sys_ppu_thread_create successfully spawns a second PPU
 *      execution unit that the scheduler picks up.
 *   2. Both threads run (child reaches its write, primary does
 *      its own write without being starved).
 *   3. sys_ppu_thread_join blocks the primary until the child
 *      calls sys_ppu_thread_exit, and delivers the exit value.
 *   4. Both threads' writes to disjoint memory regions are
 *      visible to the primary after join.
 *
 * Uses direct syscalls only (no PSL1GHT HLE wrappers) so the
 * microtest can run on CellGov without loading liblv2.sprx --
 * matches the pattern of the other microtests in this suite.
 *
 * Output layout (TTY-reported as CGOV magic + 16 bytes):
 *   status       (u32)  0 = pass, nonzero = per-step failure code
 *   child_word   (u32)  expected 0xAAAA_AAAA (child's write)
 *   parent_word  (u32)  expected 0xBBBB_BBBB (primary's write)
 *   join_retval  (u32)  expected 0xCAFE_F00D (child's exit value)
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

/* Syscall 52 takes 8 args: (thread_id*, param*, arg, unk, prio,
 * stacksize, flags, threadname*); the user-space wrapper passes
 * unk = 0. No public document states this raw-syscall argument
 * list -- it is the shape against which the microtests are built and
 * verified against. The param* in r4 is a two-word thread-init
 * block { u32 entry_opd_ptr; u32 tls }, and the OPD it names is the
 * kernel's 8-byte { u32 code; u32 toc } form. The toolchain's
 * `&fn` resolves to the function's ELFv1 .opd descriptor -- 24
 * bytes of u64 fields -- so repack it before the syscall. */
struct elfv1_opd {
    unsigned long long code;
    unsigned long long toc;
    unsigned long long env;
};

struct cg_thread_param {
    unsigned int entry_opd_ptr; /* -> opd_code below */
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

struct TestResult {
    unsigned int status;
    unsigned int child_word;
    unsigned int parent_word;
    unsigned int join_retval;
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int child_word __attribute__((aligned(128)));
static volatile unsigned int parent_word __attribute__((aligned(128)));
static struct TestResult result __attribute__((aligned(128)));

/* Child entry. Matches PSL1GHT's `void (*)(void *)` shape. Sets
 * one word and exits via direct syscall 41. */
static void child_entry(void *arg)
{
    (void)arg;
    child_word = 0xAAAAAAAA;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0xCAFEF00D);
    /* unreachable */
    for (;;) { }
}

static void write_tty_result(const struct TestResult *r)
{
    unsigned int len = sizeof(*r);
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, r, len, &written);
    CGOV_OUT_FILE_WRITE(r, len);
}

static int __attribute__((noinline)) fail(unsigned int status)
{
    result.status = status;
    result.child_word = child_word;
    result.parent_word = parent_word;
    result.join_retval = 0;
    write_tty_result(&result);
    return (int)status;
}

int main(void)
{
    unsigned long long tid = 0;
    unsigned long long retval = 0;
    s32 ret;

    /* Poison the shared words so a partial write is detectable. */
    child_word = 0xDEADBEEF;
    parent_word = 0xDEADBEEF;

    ret = cgov_ppu_thread_create(
        &tid,
        make_thread_param(&child_param, (const void *)&child_entry),
        0,          /* arg */
        0,          /* unk (reserved; liblv2's wrapper passes 0) */
        1000,       /* prio */
        0x4000,     /* stacksize */
        THREAD_JOINABLE, /* flags: the parent joins it */
        0);         /* threadname (none) */
    if (ret != 0)
        return fail(1);

    /* Parent's own write -- disjoint from the child's. Happens
     * before the join so it is not gated by the child's exit. */
    parent_word = 0xBBBBBBBB;

    /* sys_ppu_thread_join(tid, &retval). */
    ret = syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval);
    if (ret != 0)
        return fail(2);

    /* Report the observed state. Any mismatch surfaces as a
     * non-zero status field via the TTY protocol. */
    result.status = 0;
    result.child_word = child_word;
    result.parent_word = parent_word;
    result.join_retval = (unsigned int)retval;

    if (result.child_word != 0xAAAAAAAA)
        result.status |= 0x10;
    if (result.parent_word != 0xBBBBBBBB)
        result.status |= 0x20;
    if (result.join_retval != 0xCAFEF00D)
        result.status |= 0x40;

    write_tty_result(&result);
    return (int)result.status;
}
