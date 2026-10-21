/* PPU probe: whether a process can hold more than 15 MiB of child-thread
 * stacks at once, and where they lie.
 *
 * Twenty joinable threads, each with a 1 MiB stack, are all created
 * before any starts, so all twenty stacks are held together. Each
 * thread then starts, records its own sys_ppu_thread_get_stack_information
 * (49) answer and exits; main joins each in creation order. A last thread
 * of 1 MiB is then created, started and joined with every earlier one
 * collected, and records its own answer too.
 *
 * Calls: _sys_ppu_thread_create (52), sys_ppu_thread_start (53),
 * sys_ppu_thread_join (44), sys_ppu_thread_exit (41) and 49.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code
 *   2 + 5n, thread n (0-19): 52 rc, 53 rc, 44 rc, stack address, stack size
 *   102  last thread's 52 rc   103  its stack address   104  its stack size
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 105

#define SYS_PPU_THREAD_EXIT         41
#define SYS_PPU_THREAD_JOIN         44
#define SYS_PPU_THREAD_GET_STACK    49

#define THREAD_JOINABLE 1
#define CHILD_PRIO  1000
#define CHILDREN    20
#define STACK       0x100000
#define LAST        CHILDREN

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned int stack_info[CHILDREN + 1][2] __attribute__((aligned(8)));
static volatile unsigned int info_rc[CHILDREN + 1];

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

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    result[0] = step;
    result[1] = (unsigned int)rc;
    write_frame();
    return (int)step;
}

/* The toolchain's &fn is a 24-byte ELFv1 descriptor; syscall 52's
 * thread-init block names an 8-byte { u32 code; u32 toc } one. */
struct elfv1_opd {
    unsigned long long code;
    unsigned long long toc;
    unsigned long long env;
};

struct thread_param {
    unsigned int entry_opd_ptr;
    unsigned int tls;
    unsigned int opd_code;
    unsigned int opd_toc;
};

static unsigned int self_tls(void)
{
    unsigned long r13;
    __asm__ volatile ("mr %0, 13" : "=r"(r13));
    return (unsigned int)r13;
}

static u64 make_param(struct thread_param *p, const void *fn)
{
    const struct elfv1_opd *desc = (const struct elfv1_opd *)fn;
    p->opd_code = (unsigned int)desc->code;
    p->opd_toc = (unsigned int)desc->toc;
    p->entry_opd_ptr = (unsigned int)(unsigned long)&p->opd_code;
    p->tls = self_tls();
    return (u64)(unsigned long)p;
}

/* Thread n records its own stack information, then exits. */
static void child(u64 n)
{
    info_rc[n] = (unsigned int)syscall1_s32(SYS_PPU_THREAD_GET_STACK,
                                            (u64)(unsigned long)&stack_info[n][0]);
    __sync_synchronize();
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

static struct thread_param params[CHILDREN + 1] __attribute__((aligned(8)));

static unsigned int create(unsigned long long *tid, unsigned int n)
{
    return (unsigned int)syscall8_s32(CGOV_SYS_PPU_THREAD_CREATE,
                                      (u64)(unsigned long)tid,
                                      make_param(&params[n], (const void *)&child), n, 0,
                                      CHILD_PRIO, STACK, THREAD_JOINABLE, 0);
}

int main(void)
{
    unsigned int n, rc;
    unsigned long long tid[CHILDREN + 1], value;

    memset((void *)result, 0, sizeof(result));

    /* All twenty created before any starts: their stacks are held together. */
    for (n = 0; n < CHILDREN; n++) {
        rc = create(&tid[n], n);
        result[2 + 5 * n] = rc;
        if (rc != 0)
            return fail(1 + n, (int)rc);
    }
    for (n = 0; n < CHILDREN; n++) {
        rc = (unsigned int)syscall2_s32(CGOV_SYS_PPU_THREAD_START, tid[n], 0);
        result[3 + 5 * n] = rc;
        if (rc != 0)
            return fail(21 + n, (int)rc);
    }
    for (n = 0; n < CHILDREN; n++) {
        rc = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, tid[n], (u64)(unsigned long)&value);
        result[4 + 5 * n] = rc;
        if (rc != 0)
            return fail(41 + n, (int)rc);
        if (info_rc[n] != 0)
            return fail(61 + n, (int)info_rc[n]);
        result[5 + 5 * n] = stack_info[n][0];
        result[6 + 5 * n] = stack_info[n][1];
    }

    /* Every earlier thread is collected: where does the next stack go? */
    rc = create(&tid[LAST], LAST);
    result[102] = rc;
    if (rc != 0)
        return fail(81, (int)rc);
    rc = (unsigned int)syscall2_s32(CGOV_SYS_PPU_THREAD_START, tid[LAST], 0);
    if (rc != 0)
        return fail(82, (int)rc);
    rc = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, tid[LAST], (u64)(unsigned long)&value);
    if (rc != 0)
        return fail(83, (int)rc);
    if (info_rc[LAST] != 0)
        return fail(84, (int)info_rc[LAST]);
    result[103] = stack_info[LAST][0];
    result[104] = stack_info[LAST][1];

    write_frame();
    return 0;
}
