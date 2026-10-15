/* PPU program: producer-consumer bounded buffer guarded by the kernel
 * side of a lightweight mutex and two lightweight condition variables,
 * all through raw syscalls. The lwmutex-based sibling of
 * ppu_cond_prodcons.
 *
 * The kernel lwmutex is a signal queue with no signal pending at
 * create, so the program primes it with one _sys_lwmutex_unlock (98);
 * lock (97) and unlock then behave as a binary semaphore, as in
 * ppu_lwmutex_counter. A thread that holds it:
 *
 *   - waits with _sys_lwcond_queue_wait (113), which releases the
 *     lwmutex and returns 0 holding it again;
 *   - signals with _sys_lwcond_signal (115) in mode 1 (the signaller
 *     owns the lwmutex), which moves the woken waiter onto the lwmutex
 *     sleep queue; the signaller's unlock then hands it the lwmutex.
 *     With no waiter the signal answers EPERM, which is not an error
 *     here.
 *
 * Before the two threads start, the main thread records single-thread
 * answers while it holds the lwmutex.
 *
 * Output layout (TTY: CGOV magic + 64 bytes, big-endian u32 words):
 *   w0:  status, 0 = pass, a bitfield of per-check failures otherwise
 *   w1:  sum, expected N*(N-1)/2
 *   w2:  producer errors, w3: consumer errors
 *   w4:  create bound to an lwmutex id no create minted
 *   w5:  signal any thread, mode 1, no waiter
 *   w6:  signal any thread, mode 2, no waiter
 *   w7:  signal any thread, mode 3, no waiter
 *   w8:  destroy of a cond id no create minted
 *   w9:  signal a thread id no thread has, mode 1
 *   w10: signal all, mode 1, no waiter
 *   w11: signal all, mode 2, no waiter
 *   w12: queue wait with a 1 ms timeout, no signaller
 *   w13: lwmutex lock after that timeout
 *   w14: destroy of a cond with no waiter after the exchange
 *   w15: the same destroy again
 */

#include <string.h>

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PPU_THREAD_EXIT     41
#define SYS_PPU_THREAD_JOIN     44
#define SYS_LWMUTEX_CREATE      95
#define SYS_LWMUTEX_LOCK        97
#define SYS_LWMUTEX_UNLOCK      98
#define SYS_LWCOND_CREATE      111
#define SYS_LWCOND_DESTROY     112
#define SYS_LWCOND_QUEUE_WAIT  113
#define SYS_LWCOND_SIGNAL      115
#define SYS_LWCOND_SIGNAL_ALL  116

#define SYS_SYNC_FIFO 1
#define ANY_THREAD 0xFFFFFFFFu
#define NO_SUCH_ID 0x7777u
#define CELL_EPERM 0x80010009u

/* _sys_lwmutex_create (95): (id_ptr, protocol, control_ptr, has_name,
 * name). _sys_lwcond_create (111): (id_ptr, lwmutex_id, control_ptr,
 * name). Each kernel object records its user-space control block. */
static unsigned char lwmutex_control[32] __attribute__((aligned(8)));
static unsigned char lwcond_control[2][8] __attribute__((aligned(8)));

/* Syscall 52 takes (thread_id*, param*, arg, unk, prio, stacksize,
 * flags, threadname*); see ppu_cond_prodcons for the param block. */
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

static struct cg_thread_param consumer_param __attribute__((aligned(8)));

#define MESSAGES 32
#define BUF_SIZE 4

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int buffer[BUF_SIZE] __attribute__((aligned(128)));
static volatile unsigned int head_idx __attribute__((aligned(128)));
static volatile unsigned int tail_idx __attribute__((aligned(128)));
static volatile unsigned int count __attribute__((aligned(128)));

static unsigned int lwmutex_id __attribute__((aligned(128)));
static unsigned int not_empty __attribute__((aligned(128)));
static unsigned int not_full __attribute__((aligned(128)));
static unsigned int scratch_id __attribute__((aligned(128)));
static volatile unsigned int consumer_sum __attribute__((aligned(128)));
static volatile unsigned int consumer_errs_shared __attribute__((aligned(128)));
static unsigned int result[16] __attribute__((aligned(128)));

static s32 lock(void)
{
    return syscall2_s32(SYS_LWMUTEX_LOCK, lwmutex_id, 0);
}

static s32 unlock(void)
{
    return syscall1_s32(SYS_LWMUTEX_UNLOCK, lwmutex_id);
}

static s32 wait_on(unsigned int cond, unsigned long long timeout)
{
    return syscall3_s32(SYS_LWCOND_QUEUE_WAIT, cond, lwmutex_id, timeout);
}

static s32 signal(unsigned int cond, unsigned int thread, unsigned int mode)
{
    return syscall4_s32(SYS_LWCOND_SIGNAL, cond, lwmutex_id, thread, mode);
}

/* A mode-1 signal with no waiter answers EPERM: nothing to wake. */
static int signal_failed(s32 r)
{
    return r != 0 && (unsigned int)r != CELL_EPERM;
}

static void consumer_entry(void *arg)
{
    (void)arg;
    unsigned int errs = 0;
    unsigned int local_sum = 0;
    for (unsigned int i = 0; i < MESSAGES; i++) {
        if (lock() != 0) {
            errs++;
            continue;
        }
        while (count == 0) {
            if (wait_on(not_empty, 0) != 0) {
                errs++;
                break;
            }
        }
        local_sum += buffer[head_idx];
        head_idx = (head_idx + 1) % BUF_SIZE;
        count--;
        if (signal_failed(signal(not_full, ANY_THREAD, 1)))
            errs++;
        if (unlock() != 0)
            errs++;
    }
    consumer_sum = local_sum;
    consumer_errs_shared = errs;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0xBEEFCAFE);
    for (;;) { }
}

static void write_tty_result(void)
{
    unsigned int len = sizeof(result);
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, result, len, &written);
    CGOV_OUT_FILE_WRITE(result, len);
}

static int __attribute__((noinline)) fail(unsigned int status)
{
    result[0] = status;
    result[1] = consumer_sum;
    result[2] = 0xFFFFFFFF;
    result[3] = consumer_errs_shared;
    write_tty_result();
    return (int)status;
}

int main(void)
{
    unsigned long long tid = 0;
    unsigned long long retval = 0;
    s32 ret;
    unsigned int producer_errs = 0;

    memset(result, 0xEE, sizeof(result));
    consumer_sum = 0;
    consumer_errs_shared = 0xDEADBEEF;
    head_idx = 0;
    tail_idx = 0;
    count = 0;
    for (unsigned int i = 0; i < BUF_SIZE; i++)
        buffer[i] = 0;

    ret = syscall6_s32(SYS_LWMUTEX_CREATE, (unsigned long)&lwmutex_id,
                       SYS_SYNC_FIFO, (unsigned long)lwmutex_control, 0, 0, 0);
    if (ret != 0)
        return fail(0x01);
    if (unlock() != 0)
        return fail(0x02);

    ret = syscall4_s32(SYS_LWCOND_CREATE, (unsigned long)&not_empty, lwmutex_id,
                       (unsigned long)lwcond_control[0], 0);
    if (ret != 0)
        return fail(0x04);
    ret = syscall4_s32(SYS_LWCOND_CREATE, (unsigned long)&not_full, lwmutex_id,
                       (unsigned long)lwcond_control[1], 0);
    if (ret != 0)
        return fail(0x08);
    result[4] = (unsigned int)syscall4_s32(SYS_LWCOND_CREATE, (unsigned long)&scratch_id,
                                           NO_SUCH_ID, (unsigned long)lwcond_control[1], 0);

    if (lock() != 0)
        return fail(0x20);
    result[5] = (unsigned int)signal(not_empty, ANY_THREAD, 1);
    result[6] = (unsigned int)signal(not_empty, ANY_THREAD, 2);
    result[7] = (unsigned int)signal(not_empty, ANY_THREAD, 3);
    result[8] = (unsigned int)syscall1_s32(SYS_LWCOND_DESTROY, NO_SUCH_ID);
    result[9] = (unsigned int)signal(not_empty, NO_SUCH_ID, 1);
    result[10] = (unsigned int)syscall3_s32(SYS_LWCOND_SIGNAL_ALL, not_empty, lwmutex_id, 1);
    result[11] = (unsigned int)syscall3_s32(SYS_LWCOND_SIGNAL_ALL, not_empty, lwmutex_id, 2);
    result[12] = (unsigned int)wait_on(not_empty, 1000);
    result[13] = (unsigned int)lock();
    if (unlock() != 0)
        return fail(0x40);

    ret = cgov_ppu_thread_create(
        &tid,
        make_thread_param(&consumer_param, (const void *)&consumer_entry),
        0, 0, 1000, 0x4000, THREAD_JOINABLE, 0);
    if (ret != 0)
        return fail(0x80);

    for (unsigned int i = 0; i < MESSAGES; i++) {
        if (lock() != 0) {
            producer_errs++;
            continue;
        }
        while (count == BUF_SIZE) {
            if (wait_on(not_full, 0) != 0) {
                producer_errs++;
                break;
            }
        }
        buffer[tail_idx] = i;
        tail_idx = (tail_idx + 1) % BUF_SIZE;
        count++;
        if (signal_failed(signal(not_empty, ANY_THREAD, 1)))
            producer_errs++;
        if (unlock() != 0)
            producer_errs++;
    }

    ret = syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval);
    if (ret != 0)
        return fail(0x100);
    if (retval != 0xBEEFCAFE)
        return fail(0x200);

    result[14] = (unsigned int)syscall1_s32(SYS_LWCOND_DESTROY, not_full);
    result[15] = (unsigned int)syscall1_s32(SYS_LWCOND_DESTROY, not_full);

    unsigned int expected_sum = (MESSAGES * (MESSAGES - 1)) / 2;
    result[0] = 0;
    result[1] = consumer_sum;
    result[2] = producer_errs;
    result[3] = consumer_errs_shared;
    if (result[1] != expected_sum)
        result[0] |= 0x1000;
    if (result[2] != 0)
        result[0] |= 0x2000;
    if (result[3] != 0)
        result[0] |= 0x4000;
    write_tty_result();
    return (int)result[0];
}
