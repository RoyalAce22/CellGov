/* Child SELF for the process_spawn_wait microtest: write a magic
 * word into its OWN address space (the parent must never see it at
 * the same numeric address), create a PPU thread whose stack must
 * live in THIS process's address space -- the thread fills a stack
 * frame and proves the stores round-trip -- join it, burn a visible
 * number of instructions so parent polls interleave with child
 * execution, then exit 42 (CRT0 routes the return value into
 * sys_process_exit). Any earlier failure exits with a distinct
 * nonzero code the parent-side harness reports as a wrong status. */

#include <sys/process.h>

#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define MAGIC_ADDR 0x100
#define MAGIC      0x600DF00Du

#define SYS_PPU_THREAD_EXIT   41
#define SYS_PPU_THREAD_JOIN   44

/* Syscall 52's param* is a ppu_thread_param_t { u32 entry_opd_ptr;
 * u32 tls }, and the OPD it names is the kernel's 8-byte
 * { u32 code; u32 toc } form; `&fn` resolves to the ELFv1 24-byte
 * descriptor, so repack it before the syscall. */
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

static struct cg_thread_param thread_param __attribute__((aligned(8)));

/* TOC-referenced globals so the linker emits .got (the common
 * patch_toc.py step requires it). */
static volatile unsigned int spin_target = 1000;
static volatile unsigned int thread_witness = 0;

/* Sum of frame[i] = 0x51AC0000 + i for i in 0..32, mod 2^32. */
#define WITNESS_EXPECTED 0x358001F0u

/* Thread entry: fill a stack frame with distinct words and sum
 * them back, so at least 32 stack stores and loads must round-trip
 * through this process's address space before the witness lands. */
static void stack_toucher(void *arg)
{
    volatile unsigned int frame[32];
    unsigned int i;
    unsigned int sum = 0;
    (void)arg;
    for (i = 0; i < 32; i++)
        frame[i] = 0x51AC0000u + i;
    for (i = 0; i < 32; i++)
        sum += frame[i];
    thread_witness = sum;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x55);
    /* unreachable */
    for (;;) { }
}

int main(void)
{
    volatile unsigned int *magic = (volatile unsigned int *)MAGIC_ADDR;
    volatile unsigned int spin;
    unsigned long long tid = 0;
    unsigned long long thread_exit = 0;
    s32 ret;

    /* The console child (build.sh defines CGOV_PS3_USRDIR for it) skips
     * the store: 0x100 is not mapped in a console process. */
#ifndef CGOV_PS3_USRDIR
    *magic = MAGIC;
#else
    (void)magic;
#endif

    ret = cgov_ppu_thread_create(
        &tid,
        make_thread_param(&thread_param, (const void *)&stack_toucher),
        0,          /* arg */
        0,          /* unk (reserved; liblv2's wrapper passes 0) */
        1001,       /* prio */
        0x4000,     /* stacksize */
        0,          /* flags */
        0);         /* threadname (none) */
    if (ret != 0)
        return 43;

    ret = syscall8_s32(SYS_PPU_THREAD_JOIN,
                       tid,
                       (unsigned long)&thread_exit,
                       0, 0, 0, 0, 0, 0);
    if (ret != 0)
        return 44;
    if (thread_exit != 0x55)
        return 45;
    if (thread_witness != WITNESS_EXPECTED)
        return 46;

    for (spin = 0; spin < spin_target; spin++) { }
    return 42;
}
