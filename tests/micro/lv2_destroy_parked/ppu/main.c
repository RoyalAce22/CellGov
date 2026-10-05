/* PPU probe: what destroying a sync object answers while another
 * thread is parked on it, and what the parked thread gets back.
 *
 * Six cases, in payload order:
 *   0  semaphore    the waiter parks in sc 92 (sys_semaphore_wait)
 *   1  event flag   the waiter parks in sc 85 (wait, bits 1, OR)
 *   2  mutex        main holds it; the waiter parks in sc 102 (lock)
 *   3  cond         the waiter holds the mutex and parks in sc 107
 *   4  event queue  the waiter parks in sc 130; destroy mode 0
 *   5  event queue  the same; destroy mode 1 (FORCE)
 *
 * For each case:
 *   1. Create the object, and start a waiter thread at a higher
 *      priority than main. The waiter parks with a 1 s timeout, so it
 *      returns even if nothing wakes it.
 *   2. Main sleeps 20 ms (sc 141), then destroys the object: that
 *      return code is the first word of the record.
 *   3. Main sleeps 20 ms more and records whether the waiter has
 *      returned yet.
 *   4. Main joins the waiter and records the waiter's return code.
 *   5. Main releases what it holds, then destroys the object again:
 *      that return code is the last word of the record.
 *
 * Payload layout (26 big-endian u32 words, 104 bytes):
 *   +0  u32 step that failed (0 = none; 0x10 * case + 1 = a create,
 *       0x10 * case + 2 = the thread create, 0x10 * case + 3 = the
 *       join)
 *   +4  u32 that step's return code
 *   +8  six 16-byte records, one per case:
 *         +0 u32 the destroy while the waiter is parked
 *         +4 u32 1 when the waiter had returned 20 ms later, else 0
 *         +8 u32 the waiter's return code
 *         +12 u32 the destroy after the waiter returned
 */

#include <string.h>

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PPU_THREAD_EXIT      41
#define SYS_PPU_THREAD_JOIN      44
#define SYS_EVENT_FLAG_CREATE    82
#define SYS_EVENT_FLAG_DESTROY   83
#define SYS_EVENT_FLAG_WAIT      85
#define SYS_SEMAPHORE_CREATE     90
#define SYS_SEMAPHORE_DESTROY    91
#define SYS_SEMAPHORE_WAIT       92
#define SYS_MUTEX_CREATE         100
#define SYS_MUTEX_DESTROY        101
#define SYS_MUTEX_LOCK           102
#define SYS_MUTEX_UNLOCK         104
#define SYS_COND_CREATE          105
#define SYS_COND_DESTROY         106
#define SYS_COND_WAIT            107
#define SYS_EVENT_QUEUE_CREATE   128
#define SYS_EVENT_QUEUE_DESTROY  129
#define SYS_EVENT_QUEUE_RECEIVE  130
#define SYS_TIMER_USLEEP         141

#define TIMEOUT_US   1000000u
#define SETTLE_US    20000u
#define EVENT_OR     0x02
#define DESTROY_FORCE 1

#define CASE_SEMAPHORE  0
#define CASE_EVENT_FLAG 1
#define CASE_MUTEX      2
#define CASE_COND       3
#define CASE_QUEUE      4
#define CASE_QUEUE_FORCE 5
#define CASES           6

/* Attribute blocks, each field its own 32-bit word. */
static const unsigned int sem_attr[8] __attribute__((aligned(8))) =
    { 1, 0x200, 0, 0, 0, 0, 0, 0 };                    /* FIFO, private */
static const unsigned int flag_attr[8] __attribute__((aligned(8))) =
    { 1, 0x200, 0, 0, 0, 0x10000, 0, 0 };              /* FIFO, private, one waiter */
static const unsigned int mutex_attr[10] __attribute__((aligned(8))) =
    { 1, 0x20, 0x200, 0x2000, 0, 0, 0, 0, 0, 0 };      /* FIFO, not recursive */
static const unsigned int cond_attr[6] __attribute__((aligned(8))) =
    { 0x200, 0, 0, 0, 0, 0 };                           /* private */
static const unsigned int queue_attr[4] __attribute__((aligned(8))) =
    { 1, 1, 0, 0 };                                     /* FIFO, PPU queue */

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

/* Repack the toolchain's 24-byte ELFv1 descriptor into the kernel's
 * { u32 code; u32 toc } form, as ppu_event_flag_wakeall does. */
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

static struct cg_thread_param waiter_param __attribute__((aligned(8)));

struct Record {
    unsigned int destroy_parked;
    unsigned int returned_early;
    unsigned int waiter_rc;
    unsigned int destroy_after;
};

struct ProbeResult {
    unsigned int failed_step;
    unsigned int failed_rc;
    struct Record records[CASES];
};

static struct ProbeResult result __attribute__((aligned(128)));

static volatile unsigned int current_case;
static volatile unsigned int object_id;
static volatile unsigned int mutex_id;
static volatile unsigned int waiter_done;
static volatile unsigned int waiter_rc;
static unsigned int id_word __attribute__((aligned(8)));
static unsigned long long flag_result __attribute__((aligned(8)));
static unsigned long long queue_event[4] __attribute__((aligned(8)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_result(void)
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
    sysTtyWrite(0, &result, len, &written);
    CGOV_OUT_FILE_WRITE(&result, len);
}

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    result.failed_step = step;
    result.failed_rc = (unsigned int)rc;
    write_result();
    return (int)step;
}

#define EA(p) ((unsigned long)(const void *)(p))

static void waiter_entry(void *arg)
{
    s32 rc;
    (void)arg;
    switch (current_case) {
    case CASE_SEMAPHORE:
        rc = syscall2_s32(SYS_SEMAPHORE_WAIT, object_id, TIMEOUT_US);
        break;
    case CASE_EVENT_FLAG:
        rc = syscall5_s32(SYS_EVENT_FLAG_WAIT, object_id, 1, EVENT_OR,
                          EA(&flag_result), TIMEOUT_US);
        break;
    case CASE_MUTEX:
        rc = syscall2_s32(SYS_MUTEX_LOCK, object_id, TIMEOUT_US);
        if (rc == 0)
            (void)syscall1_s32(SYS_MUTEX_UNLOCK, object_id);
        break;
    case CASE_COND:
        rc = syscall2_s32(SYS_MUTEX_LOCK, mutex_id, 0);
        if (rc == 0) {
            rc = syscall2_s32(SYS_COND_WAIT, object_id, TIMEOUT_US);
            (void)syscall1_s32(SYS_MUTEX_UNLOCK, mutex_id);
        }
        break;
    default:
        rc = syscall3_s32(SYS_EVENT_QUEUE_RECEIVE, object_id, EA(queue_event), TIMEOUT_US);
        break;
    }
    waiter_rc = (unsigned int)rc;
    waiter_done = 1;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

static s32 create_object(unsigned int c)
{
    s32 rc;
    id_word = 0;
    switch (c) {
    case CASE_SEMAPHORE:
        rc = syscall4_s32(SYS_SEMAPHORE_CREATE, EA(&id_word), EA(sem_attr), 0, 1);
        break;
    case CASE_EVENT_FLAG:
        rc = syscall3_s32(SYS_EVENT_FLAG_CREATE, EA(&id_word), EA(flag_attr), 0);
        break;
    case CASE_MUTEX:
        rc = syscall2_s32(SYS_MUTEX_CREATE, EA(&id_word), EA(mutex_attr));
        break;
    case CASE_COND:
        rc = syscall2_s32(SYS_MUTEX_CREATE, EA(&id_word), EA(mutex_attr));
        if (rc != 0)
            return rc;
        mutex_id = id_word;
        id_word = 0;
        rc = syscall3_s32(SYS_COND_CREATE, EA(&id_word), mutex_id, EA(cond_attr));
        break;
    default:
        rc = syscall4_s32(SYS_EVENT_QUEUE_CREATE, EA(&id_word), EA(queue_attr), 0, 16);
        break;
    }
    object_id = id_word;
    return rc;
}

static s32 destroy_object(unsigned int c)
{
    switch (c) {
    case CASE_SEMAPHORE:   return syscall1_s32(SYS_SEMAPHORE_DESTROY, object_id);
    case CASE_EVENT_FLAG:  return syscall1_s32(SYS_EVENT_FLAG_DESTROY, object_id);
    case CASE_MUTEX:       return syscall1_s32(SYS_MUTEX_DESTROY, object_id);
    case CASE_COND:        return syscall1_s32(SYS_COND_DESTROY, object_id);
    case CASE_QUEUE:       return syscall2_s32(SYS_EVENT_QUEUE_DESTROY, object_id, 0);
    default:               return syscall2_s32(SYS_EVENT_QUEUE_DESTROY, object_id, DESTROY_FORCE);
    }
}

static int run_case(unsigned int c)
{
    unsigned long long tid = 0, retval = 0;
    struct Record *rec = &result.records[c];
    s32 rc;

    current_case = c;
    rc = create_object(c);
    if (rc != 0)
        return fail(0x10 * c + 1, rc);
    if (c == CASE_MUTEX)
        (void)syscall2_s32(SYS_MUTEX_LOCK, object_id, 0);

    waiter_done = 0;
    waiter_rc = 0xFFFFFFFFu;
    rc = cgov_ppu_thread_create(&tid, make_thread_param(&waiter_param, (const void *)&waiter_entry),
                                0, 0, 1000, 0x4000, THREAD_JOINABLE, 0);
    if (rc != 0)
        return fail(0x10 * c + 2, rc);

    (void)syscall1_s32(SYS_TIMER_USLEEP, SETTLE_US);
    rec->destroy_parked = (unsigned int)destroy_object(c);
    (void)syscall1_s32(SYS_TIMER_USLEEP, SETTLE_US);
    rec->returned_early = waiter_done;

    rc = syscall2_s32(SYS_PPU_THREAD_JOIN, tid, EA(&retval));
    if (rc != 0)
        return fail(0x10 * c + 3, rc);
    rec->waiter_rc = waiter_rc;

    if (c == CASE_MUTEX)
        (void)syscall1_s32(SYS_MUTEX_UNLOCK, object_id);
    rec->destroy_after = (unsigned int)destroy_object(c == CASE_QUEUE_FORCE ? CASE_QUEUE : c);
    if (c == CASE_COND)
        (void)syscall1_s32(SYS_MUTEX_DESTROY, mutex_id);
    return 0;
}

int main(void)
{
    unsigned int c;
    memset(&result, 0, sizeof(result));
    for (c = 0; c < CASES; c++)
        if (run_case(c) != 0)
            return 1;
    write_result();
    return 0;
}
