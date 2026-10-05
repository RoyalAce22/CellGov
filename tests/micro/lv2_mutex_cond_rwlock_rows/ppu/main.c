/* PPU probe: the lwmutex, mutex, cond and rwlock rows the behavior
 * table marks unestablished.
 *
 * Steps and payload words (big-endian u32):
 *   A. lwmutex ids
 *     0-2   sc 95 create, three times (each with its own control block)
 *     3-5   the three ids
 *     6-8   sc 96 destroy each
 *   B. rwlock
 *     9     sc 121(0xDEADBEEF)
 *     10    sc 120 create      11  its id
 *     12    sc 121 destroy     13  sc 121 destroy again
 *   C. mutex trylock while another thread holds it
 *     14    sc 100 create
 *     15    sc 103 while the helper thread holds the mutex
 *     16    sc 103 after the helper released it
 *     17    sc 104 unlock
 *     18    sc 101 destroy     19  sc 101 destroy again
 *   D. cond signal while main holds the mutex
 *     20    sc 100 mutex       21  sc 105 cond
 *     22    sc 102 main locks the mutex (the waiter is parked in sc 107)
 *     23    sc 108 signal
 *     24    1 when the waiter had returned 20 ms after the signal
 *     25    sc 104 main unlocks
 *     26    1 when the waiter had returned 20 ms after the unlock
 *     27    the waiter's sc 107 return code
 *   E. signal_all with two waiters
 *     28    sc 109
 *     29    waiters returned 40 ms later (0, 1 or 2)
 *     30,31 the two waiters' return codes
 *   F. signal_to
 *     32    sc 110 to the parked waiter's thread id
 *     33    sc 110 to thread id 0xDEADBEEF
 *     34    the waiter's return code
 *   G. no waiter, stale ids, cleanup
 *     35    sc 108 with no waiter
 *     36    sc 108(0xDEADBEEF)
 *     37    sc 106 cond destroy   38  sc 101 mutex destroy
 *     39    step that failed (0 = none; see fail())
 *
 * Every waiter parks with a 1 s timeout, so none can hang the probe.
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
#define SYS_LWMUTEX_CREATE       95
#define SYS_LWMUTEX_DESTROY      96
#define SYS_MUTEX_CREATE         100
#define SYS_MUTEX_DESTROY        101
#define SYS_MUTEX_LOCK           102
#define SYS_MUTEX_TRYLOCK        103
#define SYS_MUTEX_UNLOCK         104
#define SYS_COND_CREATE          105
#define SYS_COND_DESTROY         106
#define SYS_COND_WAIT            107
#define SYS_COND_SIGNAL          108
#define SYS_COND_SIGNAL_ALL      109
#define SYS_COND_SIGNAL_TO       110
#define SYS_RWLOCK_CREATE        120
#define SYS_RWLOCK_DESTROY       121
#define SYS_TIMER_USLEEP         141

#define TIMEOUT_US 1000000u
#define SETTLE_US  20000u

#define ROLE_HOLDER 0
#define ROLE_WAITER 1

static const unsigned int mutex_attr[10] __attribute__((aligned(8))) =
    { 1, 0x20, 0x200, 0x2000, 0, 0, 0, 0, 0, 0 };   /* FIFO, not recursive */
static const unsigned int cond_attr[6] __attribute__((aligned(8))) =
    { 0x200, 0, 0, 0, 0, 0 };                        /* private */
/* sys_rwlock_attribute_t: protocol@0, pshared@4, ipc_key@8 (u64),
 * flags@16, pad@20, name@24. */
static const unsigned int rwlock_attr[8] __attribute__((aligned(8))) =
    { 1, 0x200, 0, 0, 0, 0, 0, 0 };

static unsigned char lw_control[3][32] __attribute__((aligned(8)));

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

/* Repack the 24-byte ELFv1 descriptor into the kernel's 8-byte form,
 * as ppu_event_flag_wakeall does. */
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

static struct cg_thread_param params[2] __attribute__((aligned(8)));

static unsigned int result[40] __attribute__((aligned(128)));
static unsigned int id_word __attribute__((aligned(8)));
static volatile unsigned int mutex_id, cond_id;
static volatile unsigned int release_holder, holding;
static volatile unsigned int done[2], rcs[2];

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
    sysTtyWrite(0, result, len, &written);
    CGOV_OUT_FILE_WRITE(result, len);
}

static int __attribute__((noinline)) fail(unsigned int step)
{
    result[39] = step;
    write_result();
    return (int)step;
}

#define EA(p) ((unsigned long)(const void *)(p))

static void usleep_us(unsigned int us)
{
    (void)syscall1_s32(SYS_TIMER_USLEEP, us);
}

/* The holder locks the mutex, holds it until main sets
 * release_holder, then unlocks. A waiter (slot = arg - ROLE_WAITER)
 * locks the mutex, parks in sc 107, and unlocks after it returns. */
static void worker(void *arg)
{
    unsigned long role = (unsigned long)arg;
    if (role == ROLE_HOLDER) {
        (void)syscall2_s32(SYS_MUTEX_LOCK, mutex_id, 0);
        holding = 1;
        while (!release_holder)
            usleep_us(1000);
        (void)syscall1_s32(SYS_MUTEX_UNLOCK, mutex_id);
    } else {
        unsigned int slot = (unsigned int)(role - ROLE_WAITER);
        s32 rc = syscall2_s32(SYS_MUTEX_LOCK, mutex_id, 0);
        if (rc == 0) {
            rc = syscall2_s32(SYS_COND_WAIT, cond_id, TIMEOUT_US);
            (void)syscall1_s32(SYS_MUTEX_UNLOCK, mutex_id);
        }
        rcs[slot] = (unsigned int)rc;
        done[slot] = 1;
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

static s32 start(unsigned long long *tid, unsigned int slot, unsigned long role)
{
    return cgov_ppu_thread_create(tid, make_thread_param(&params[slot], (const void *)&worker),
                                  role, 0, 1000, 0x4000, THREAD_JOINABLE, 0);
}

static s32 join(unsigned long long tid)
{
    unsigned long long retval = 0;
    return syscall2_s32(SYS_PPU_THREAD_JOIN, tid, EA(&retval));
}

static void reset_waiters(void)
{
    done[0] = done[1] = 0;
    rcs[0] = rcs[1] = 0xFFFFFFFFu;
}

int main(void)
{
    unsigned long long t0 = 0, t1 = 0;
    unsigned int i, ids[3];

    memset(result, 0, sizeof(result));

    /* A. */
    for (i = 0; i < 3; i++) {
        id_word = 0;
        result[i] = (unsigned int)syscall6_s32(SYS_LWMUTEX_CREATE, EA(&id_word), 1,
                                               EA(lw_control[i]), 0, 0, 0);
        ids[i] = id_word;
        result[3 + i] = id_word;
    }
    for (i = 0; i < 3; i++)
        result[6 + i] = (unsigned int)syscall1_s32(SYS_LWMUTEX_DESTROY, ids[i]);

    /* B. */
    result[9] = (unsigned int)syscall1_s32(SYS_RWLOCK_DESTROY, 0xDEADBEEFu);
    id_word = 0;
    result[10] = (unsigned int)syscall2_s32(SYS_RWLOCK_CREATE, EA(&id_word), EA(rwlock_attr));
    result[11] = id_word;
    result[12] = (unsigned int)syscall1_s32(SYS_RWLOCK_DESTROY, id_word);
    result[13] = (unsigned int)syscall1_s32(SYS_RWLOCK_DESTROY, result[11]);

    /* C. */
    id_word = 0;
    result[14] = (unsigned int)syscall2_s32(SYS_MUTEX_CREATE, EA(&id_word), EA(mutex_attr));
    mutex_id = id_word;
    holding = 0;
    release_holder = 0;
    if (start(&t0, 0, ROLE_HOLDER) != 0)
        return fail(0x31);
    while (!holding)
        usleep_us(1000);
    result[15] = (unsigned int)syscall1_s32(SYS_MUTEX_TRYLOCK, mutex_id);
    release_holder = 1;
    if (join(t0) != 0)
        return fail(0x32);
    result[16] = (unsigned int)syscall1_s32(SYS_MUTEX_TRYLOCK, mutex_id);
    result[17] = (unsigned int)syscall1_s32(SYS_MUTEX_UNLOCK, mutex_id);
    result[18] = (unsigned int)syscall1_s32(SYS_MUTEX_DESTROY, mutex_id);
    result[19] = (unsigned int)syscall1_s32(SYS_MUTEX_DESTROY, mutex_id);

    /* D. */
    id_word = 0;
    result[20] = (unsigned int)syscall2_s32(SYS_MUTEX_CREATE, EA(&id_word), EA(mutex_attr));
    mutex_id = id_word;
    id_word = 0;
    result[21] = (unsigned int)syscall3_s32(SYS_COND_CREATE, EA(&id_word), mutex_id,
                                            EA(cond_attr));
    cond_id = id_word;
    reset_waiters();
    if (start(&t0, 0, ROLE_WAITER) != 0)
        return fail(0x41);
    usleep_us(SETTLE_US);
    result[22] = (unsigned int)syscall2_s32(SYS_MUTEX_LOCK, mutex_id, 0);
    result[23] = (unsigned int)syscall1_s32(SYS_COND_SIGNAL, cond_id);
    usleep_us(SETTLE_US);
    result[24] = done[0];
    result[25] = (unsigned int)syscall1_s32(SYS_MUTEX_UNLOCK, mutex_id);
    usleep_us(SETTLE_US);
    result[26] = done[0];
    if (join(t0) != 0)
        return fail(0x42);
    result[27] = rcs[0];

    /* E. */
    reset_waiters();
    if (start(&t0, 0, ROLE_WAITER) != 0 || start(&t1, 1, ROLE_WAITER + 1) != 0)
        return fail(0x51);
    usleep_us(SETTLE_US);
    result[28] = (unsigned int)syscall1_s32(SYS_COND_SIGNAL_ALL, cond_id);
    usleep_us(2 * SETTLE_US);
    result[29] = done[0] + done[1];
    if (join(t0) != 0 || join(t1) != 0)
        return fail(0x52);
    result[30] = rcs[0];
    result[31] = rcs[1];

    /* F. */
    reset_waiters();
    if (start(&t0, 0, ROLE_WAITER) != 0)
        return fail(0x61);
    usleep_us(SETTLE_US);
    result[32] = (unsigned int)syscall2_s32(SYS_COND_SIGNAL_TO, cond_id, (unsigned int)t0);
    result[33] = (unsigned int)syscall2_s32(SYS_COND_SIGNAL_TO, cond_id, 0xDEADBEEFu);
    if (join(t0) != 0)
        return fail(0x62);
    result[34] = rcs[0];

    /* G. */
    result[35] = (unsigned int)syscall1_s32(SYS_COND_SIGNAL, cond_id);
    result[36] = (unsigned int)syscall1_s32(SYS_COND_SIGNAL, 0xDEADBEEFu);
    result[37] = (unsigned int)syscall1_s32(SYS_COND_DESTROY, cond_id);
    result[38] = (unsigned int)syscall1_s32(SYS_MUTEX_DESTROY, mutex_id);

    write_result();
    return 0;
}
