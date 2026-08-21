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

SYS_PROCESS_PARAM(1001, 0x10000)

#define MAGIC_ADDR 0x100
#define MAGIC      0x600DF00Du

#define SYS_PPU_THREAD_EXIT   41
#define SYS_PPU_THREAD_JOIN   44
#define SYS_PPU_THREAD_CREATE 52

/* Direct-syscall helpers, same shape as the sync-primitive
 * microtests: r11 = syscall number, r3-r10 = args, `sc`, r3 =
 * return value. */
static inline s32 syscall8_s32(u64 num, u64 a, u64 b, u64 c, u64 d,
                               u64 e, u64 f, u64 g, u64 h)
{
    register u64 r3 __asm__("3") = a;
    register u64 r4 __asm__("4") = b;
    register u64 r5 __asm__("5") = c;
    register u64 r6 __asm__("6") = d;
    register u64 r7 __asm__("7") = e;
    register u64 r8 __asm__("8") = f;
    register u64 r9 __asm__("9") = g;
    register u64 r10 __asm__("10") = h;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile (
        "sc\n"
        : "+r"(r3)
        : "r"(r4), "r"(r5), "r"(r6), "r"(r7), "r"(r8), "r"(r9), "r"(r10), "r"(r11)
        : "r0", "r12", "cr0", "ctr", "memory"
    );
    return (s32)r3;
}

static inline void syscall1_noreturn(u64 num, u64 a)
{
    register u64 r3 __asm__("3") = a;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile (
        "sc\n"
        :
        : "r"(r3), "r"(r11)
        : "memory"
    );
}

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

    *magic = MAGIC;

    ret = syscall8_s32(
        SYS_PPU_THREAD_CREATE,
        (unsigned long)&tid,
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
