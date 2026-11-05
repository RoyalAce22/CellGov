/* PPU probe: the round trip of LV2 calls refused for a null result
 * pointer, and of a mutex trylock refused because another thread holds
 * the mutex, each timed with mftb.
 *
 * Each loop makes 1 << 12 calls of one syscall, five runs a loop. The
 * probe creates two semaphores (count 0, maximum 1), one event flag
 * (value 0), one event queue (empty), one non-recursive mutex and one
 * joinable PPU thread, and destroys or joins all six before it writes its
 * result. No timed call blocks.
 *
 * The helper thread locks the mutex, posts the first semaphore, and waits
 * on the second with a 10 s timeout; main times the trylock loop while
 * the helper holds the mutex, then posts the second semaphore and joins.
 *
 *   loop
 *    0  sc 147 sys_time_get_timebase_frequency, the baseline
 *    1  sc 145 sys_time_get_current_time, both pointers null
 *    2  sc 352 sys_memory_get_user_memory_size, null pointer
 *    3  sc 114 sys_semaphore_get_value, live semaphore, null pointer
 *    4  sc 139 sys_event_flag_get, live event flag, null pointer
 *    5  sc 131 sys_event_queue_tryreceive, live queue, null array and count
 *    6  sc 103 sys_mutex_trylock, the mutex the helper thread holds
 *
 *   word
 *   2L, 2L+1   loop L's fastest and slowest run, in time-base ticks
 *   16 + L     loop L's first nonzero return code, or 0
 *   24 + L     loop L's last return code, as the low 32 bits of r3
 *   32  first semaphore create rc     33  second semaphore create rc
 *   34  event flag create rc          35  event queue create rc
 *   36  mutex create rc               37  thread create rc
 *   38  main's wait on the first semaphore, rc
 *   39  the helper's lock of the mutex, rc
 *   40  main's post of the second semaphore, rc
 *   41  the join, rc
 *   42  the helper's wait on the second semaphore, rc
 *   43  the helper's unlock of the mutex, rc
 *   44  mutex destroy rc              45  event queue destroy rc
 *   46  event flag destroy rc         47  second semaphore destroy rc
 *   48  first semaphore destroy rc
 *   57  the first setup or teardown step whose rc was nonzero: its word
 *       above, or 0 when every step returned 0
 *   59  runs a loop (5)
 *   60  0x5050_4C41 when the probe wrote every word
 *
 * A loop whose setup failed is skipped and its words stay 0.
 *
 * Payload: 64 u64 words, 512 bytes.
 */

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PPU_THREAD_EXIT              41
#define SYS_PPU_THREAD_JOIN              44
#define SYS_EVENT_FLAG_CREATE            82
#define SYS_EVENT_FLAG_DESTROY           83
#define SYS_SEMAPHORE_CREATE             90
#define SYS_SEMAPHORE_DESTROY            91
#define SYS_SEMAPHORE_WAIT               92
#define SYS_SEMAPHORE_POST               94
#define SYS_MUTEX_CREATE                 100
#define SYS_MUTEX_DESTROY                101
#define SYS_MUTEX_LOCK                   102
#define SYS_MUTEX_TRYLOCK                103
#define SYS_MUTEX_UNLOCK                 104
#define SYS_SEMAPHORE_GET_VALUE          114
#define SYS_EVENT_QUEUE_CREATE           128
#define SYS_EVENT_QUEUE_DESTROY          129
#define SYS_EVENT_QUEUE_TRYRECEIVE       131
#define SYS_EVENT_FLAG_GET               139
#define SYS_TIME_GET_CURRENT_TIME        145
#define SYS_TIME_GET_TIMEBASE_FREQUENCY  147
#define SYS_MEMORY_GET_USER_MEMORY_SIZE  352

/* Main's wait for the helper, and the helper's lock and wait, in us. */
#define HELD_TIMEOUT_US     1000000u
#define LOCK_TIMEOUT_US     1000000u
#define RELEASE_TIMEOUT_US 10000000u
#define HELPER_PRIO   1000
#define HELPER_STACK  0x4000

#define WORDS   64
#define PAYLOAD (WORDS * 8)
#define REPS    5
#define CALLS   (1u << 12)
#define LOOPS   7
#define DONE    0x50504C41ull

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

/* Attribute blocks, each field its own 32-bit word. */
static const unsigned int sem_attr[8] __attribute__((aligned(8))) =
    { 1, 0x200, 0, 0, 0, 0, 0, 0 };                    /* FIFO, private */
static const unsigned int flag_attr[8] __attribute__((aligned(8))) =
    { 1, 0x200, 0, 0, 0, 0x10000, 0, 0 };              /* FIFO, private, one waiter */
static const unsigned int mutex_attr[10] __attribute__((aligned(8))) =
    { 1, 0x20, 0x200, 0x2000, 0, 0, 0, 0, 0, 0 };      /* FIFO, non-recursive */
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
 * { u32 code; u32 toc } form, as lv2_destroy_parked does. */
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

static struct cg_thread_param helper_param __attribute__((aligned(8)));

static volatile unsigned int id_word __attribute__((aligned(8)));
static volatile unsigned int held_sem, release_sem, mutex;
static volatile s32 helper_lock_rc = -1, helper_wait_rc = -1, helper_unlock_rc = -1;

static u64 first_rc[LOOPS];
static u64 last_rc[LOOPS];
static u64 first_failed;

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static inline u64 timebase(void)
{
    u64 tb;
    __asm__ volatile ("mftb %0" : "=r"(tb));
    return tb;
}

static void put(unsigned slot, u64 v)
{
    unsigned i;
    for (i = 0; i < 8; i++)
        out[slot * 8 + i] = (unsigned char)(v >> (56 - 8 * i));
}

/* Record a setup or teardown step's rc at `slot`; keep the first that
 * failed. */
static u64 step(unsigned slot, u64 rc)
{
    put(slot, rc & 0xFFFFFFFFull);
    if ((rc & 0xFFFFFFFFull) && !first_failed)
        first_failed = slot;
    return rc & 0xFFFFFFFFull;
}

static void write_frame(void)
{
    unsigned int written;
    unsigned int len = PAYLOAD;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)out, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)out, len);
}

/* CALLS calls of syscall `num`; returns the time-base ticks they took.
 * Keeps the first nonzero and the last return code of the loop. */
static u64 __attribute__((noinline)) calls(unsigned loop, u64 num, u64 a, u64 b, u64 c, u64 d)
{
    u64 i, rc = 0, bad = 0;
    u64 t0 = timebase();
    for (i = 0; i < CALLS; i++) {
        rc = (u64)cgov_sc(num, a, b, c, d, 0, 0, 0, 0);
        bad = bad ? bad : (rc & 0xFFFFFFFFull);
    }
    t0 = timebase() - t0;
    if (bad && !first_rc[loop])
        first_rc[loop] = bad;
    last_rc[loop] = rc & 0xFFFFFFFFull;
    return t0;
}

/* Run loop `loop` REPS times; keep its fastest and slowest run. */
static void timed(unsigned loop, u64 num, u64 a, u64 b, u64 c, u64 d)
{
    u64 lo = ~0ull, hi = 0;
    unsigned r;
    for (r = 0; r < REPS; r++) {
        u64 t = calls(loop, num, a, b, c, d);
        if (t < lo)
            lo = t;
        if (t > hi)
            hi = t;
    }
    put(2 * loop, lo);
    put(2 * loop + 1, hi);
}

/* The helper: hold the mutex until main posts the second semaphore. The
 * first semaphore is posted whether or not the lock succeeded, so main
 * never waits out its timeout on a helper that failed. */
static void helper_entry(void *arg)
{
    (void)arg;
    helper_lock_rc = syscall2_s32(SYS_MUTEX_LOCK, mutex, LOCK_TIMEOUT_US);
    (void)syscall2_s32(SYS_SEMAPHORE_POST, held_sem, 1);
    helper_wait_rc = syscall2_s32(SYS_SEMAPHORE_WAIT, release_sem, RELEASE_TIMEOUT_US);
    if (helper_lock_rc == 0)
        helper_unlock_rc = syscall1_s32(SYS_MUTEX_UNLOCK, mutex);
    else
        helper_unlock_rc = 0;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

int main(void)
{
    unsigned i;
    u64 held_rc, release_rc, flag_rc, queue_rc, mutex_rc;
    unsigned int flag = 0, queue = 0;
    unsigned long long tid = 0, retval = 0;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    held_rc = step(32, (u64)cgov_sc(SYS_SEMAPHORE_CREATE, EA(&id_word), EA(sem_attr), 0, 1, 0, 0, 0, 0));
    held_sem = id_word;
    release_rc = step(33, (u64)cgov_sc(SYS_SEMAPHORE_CREATE, EA(&id_word), EA(sem_attr), 0, 1, 0, 0, 0, 0));
    release_sem = id_word;
    flag_rc = step(34, (u64)cgov_sc(SYS_EVENT_FLAG_CREATE, EA(&id_word), EA(flag_attr), 0, 0, 0, 0, 0, 0));
    flag = id_word;
    queue_rc = step(35, (u64)cgov_sc(SYS_EVENT_QUEUE_CREATE, EA(&id_word), EA(queue_attr), 0, 16, 0, 0, 0, 0));
    queue = id_word;
    mutex_rc = step(36, (u64)cgov_sc(SYS_MUTEX_CREATE, EA(&id_word), EA(mutex_attr), 0, 0, 0, 0, 0, 0));
    mutex = id_word;

    timed(0, SYS_TIME_GET_TIMEBASE_FREQUENCY, 0, 0, 0, 0);
    timed(1, SYS_TIME_GET_CURRENT_TIME, 0, 0, 0, 0);
    timed(2, SYS_MEMORY_GET_USER_MEMORY_SIZE, 0, 0, 0, 0);
    if (held_rc == 0)
        timed(3, SYS_SEMAPHORE_GET_VALUE, held_sem, 0, 0, 0);
    if (flag_rc == 0)
        timed(4, SYS_EVENT_FLAG_GET, flag, 0, 0, 0);
    if (queue_rc == 0)
        timed(5, SYS_EVENT_QUEUE_TRYRECEIVE, queue, 0, 1, 0);

    if (held_rc == 0 && release_rc == 0 && mutex_rc == 0) {
        s32 rc = cgov_ppu_thread_create(&tid, make_thread_param(&helper_param, (const void *)&helper_entry),
                                        0, 0, HELPER_PRIO, HELPER_STACK, THREAD_JOINABLE, 0);
        if (step(37, (u64)rc) == 0) {
            if (step(38, (u64)cgov_sc(SYS_SEMAPHORE_WAIT, held_sem, HELD_TIMEOUT_US, 0, 0, 0, 0, 0, 0)) == 0
                && helper_lock_rc == 0)
                timed(6, SYS_MUTEX_TRYLOCK, mutex, 0, 0, 0);
            step(40, (u64)cgov_sc(SYS_SEMAPHORE_POST, release_sem, 1, 0, 0, 0, 0, 0, 0));
            step(41, (u64)cgov_sc(SYS_PPU_THREAD_JOIN, tid, EA(&retval), 0, 0, 0, 0, 0, 0));
            step(39, (u64)(unsigned int)helper_lock_rc);
            step(42, (u64)(unsigned int)helper_wait_rc);
            step(43, (u64)(unsigned int)helper_unlock_rc);
        }
    }

    if (mutex_rc == 0)
        step(44, (u64)cgov_sc(SYS_MUTEX_DESTROY, mutex, 0, 0, 0, 0, 0, 0, 0));
    if (queue_rc == 0)
        step(45, (u64)cgov_sc(SYS_EVENT_QUEUE_DESTROY, queue, 0, 0, 0, 0, 0, 0, 0));
    if (flag_rc == 0)
        step(46, (u64)cgov_sc(SYS_EVENT_FLAG_DESTROY, flag, 0, 0, 0, 0, 0, 0, 0));
    if (release_rc == 0)
        step(47, (u64)cgov_sc(SYS_SEMAPHORE_DESTROY, release_sem, 0, 0, 0, 0, 0, 0, 0));
    if (held_rc == 0)
        step(48, (u64)cgov_sc(SYS_SEMAPHORE_DESTROY, held_sem, 0, 0, 0, 0, 0, 0, 0));

    for (i = 0; i < LOOPS; i++) {
        put(16 + i, first_rc[i]);
        put(24 + i, last_rc[i]);
    }
    put(57, first_failed);
    put(59, REPS);
    put(60, DONE);
    write_frame();
    return 0;
}
