/* PPU probe: the round trip of LV2 calls that find their object and then
 * refuse the call, each timed with mftb.
 *
 * Each loop makes 1 << 12 calls of one syscall, five runs a loop. The
 * probe creates one semaphore (count 0, maximum 1), one non-recursive
 * mutex and one event flag (value 0), and destroys all three before it
 * writes its result. No call blocks: every timed call is refused at once,
 * and the mutex lock carries a timeout in case it is not.
 *
 *   loop
 *    0  sc 147 sys_time_get_timebase_frequency, the baseline
 *    1  sc 93  sys_semaphore_trywait, the count 0          -> EBUSY
 *    2  sc 94  sys_semaphore_post 1, the count at maximum  -> EBUSY
 *    3  sc 94  sys_semaphore_post -1                       -> EINVAL
 *    4  sc 102 sys_mutex_lock, held by this thread         -> EDEADLK
 *    5  sc 103 sys_mutex_trylock, held by this thread      -> EDEADLK
 *    6  sc 104 sys_mutex_unlock, held by no thread         -> EPERM
 *    7  sc 86  sys_event_flag_trywait bit 0 AND, value 0   -> EBUSY
 *
 *   word
 *   2L, 2L+1   loop L's fastest and slowest run, in time-base ticks
 *   32 + L     loop L's first nonzero return code, or 0
 *   48  semaphore create rc    49  mutex create rc
 *   50  event flag create rc   51  the post that fills the semaphore, rc
 *   52  the mutex lock before loop 4, rc
 *   53  the mutex unlock after loop 5, rc
 *   54  semaphore destroy rc   55  mutex destroy rc
 *   56  event flag destroy rc
 *   57  the first setup or teardown step whose rc was nonzero: its word
 *       above, or 0 when every step returned 0
 *   59  runs a loop (5)
 *   60  0x5050_4C45 when the probe wrote every word
 *
 * A loop whose setup failed is skipped and its words stay 0.
 *
 * Payload: 64 u64 words, 512 bytes.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_EVENT_FLAG_CREATE            82
#define SYS_EVENT_FLAG_DESTROY           83
#define SYS_EVENT_FLAG_TRYWAIT           86
#define SYS_SEMAPHORE_CREATE             90
#define SYS_SEMAPHORE_DESTROY            91
#define SYS_SEMAPHORE_TRYWAIT            93
#define SYS_SEMAPHORE_POST               94
#define SYS_MUTEX_CREATE                 100
#define SYS_MUTEX_DESTROY                101
#define SYS_MUTEX_LOCK                   102
#define SYS_MUTEX_TRYLOCK                103
#define SYS_MUTEX_UNLOCK                 104
#define SYS_TIME_GET_TIMEBASE_FREQUENCY  147

#define FIFO               0x1
#define NOT_RECURSIVE      0x20
#define NOT_PROCESS_SHARED 0x200
#define NOT_ADAPTIVE       0x2000

/* sys_event_flag_trywait's AND mode. */
#define WAIT_AND    0x1
/* The mutex lock's timeout, in microseconds, should a lock block. */
#define LOCK_TIMEOUT_US 100

#define WORDS   64
#define PAYLOAD (WORDS * 8)
#define REPS    5
#define CALLS   (1u << 12)
#define LOOPS   8
#define DONE    0x50504C45ull

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned int mutex_attr[10] __attribute__((aligned(8)));
static volatile unsigned int sem_attr[8] __attribute__((aligned(8)));
static const unsigned int flag_attr[8] __attribute__((aligned(8))) =
    { 1, 0x200, 0, 0, 0, 0x10000, 0, 0 };   /* FIFO, private, one waiter */

static volatile unsigned int id_word __attribute__((aligned(8)));
static volatile u64 flag_result __attribute__((aligned(8)));

static u64 first_rc[LOOPS];
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
    put(slot, rc);
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
 * Keeps the first nonzero return code of the loop. */
static u64 __attribute__((noinline)) calls(unsigned loop, u64 num, u64 a, u64 b, u64 c, u64 d)
{
    u64 i, rc, bad = 0;
    u64 t0 = timebase();
    for (i = 0; i < CALLS; i++) {
        rc = (u64)cgov_sc(num, a, b, c, d, 0, 0, 0, 0);
        bad = bad ? bad : (rc & 0xFFFFFFFFull);
    }
    t0 = timebase() - t0;
    if (bad && !first_rc[loop])
        first_rc[loop] = bad;
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

int main(void)
{
    unsigned i;
    u64 sem_rc, mutex_rc, flag_rc;
    unsigned int sem = 0, mutex = 0, flag = 0;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < 10; i++)
        mutex_attr[i] = 0;
    mutex_attr[0] = FIFO;
    mutex_attr[1] = NOT_RECURSIVE;
    mutex_attr[2] = NOT_PROCESS_SHARED;
    mutex_attr[3] = NOT_ADAPTIVE;
    for (i = 0; i < 8; i++)
        sem_attr[i] = 0;
    sem_attr[0] = FIFO;
    sem_attr[1] = NOT_PROCESS_SHARED;

    sem_rc = step(48, (u64)cgov_sc(SYS_SEMAPHORE_CREATE, EA(&id_word), EA(sem_attr), 0, 1, 0, 0, 0, 0));
    sem = id_word;
    mutex_rc = step(49, (u64)cgov_sc(SYS_MUTEX_CREATE, EA(&id_word), EA(mutex_attr), 0, 0, 0, 0, 0, 0));
    mutex = id_word;
    flag_rc = step(50, (u64)cgov_sc(SYS_EVENT_FLAG_CREATE, EA(&id_word), EA(flag_attr), 0, 0, 0, 0, 0, 0));
    flag = id_word;

    timed(0, SYS_TIME_GET_TIMEBASE_FREQUENCY, 0, 0, 0, 0);

    if (sem_rc == 0) {
        timed(1, SYS_SEMAPHORE_TRYWAIT, sem, 0, 0, 0);
        if (step(51, (u64)cgov_sc(SYS_SEMAPHORE_POST, sem, 1, 0, 0, 0, 0, 0, 0)) == 0)
            timed(2, SYS_SEMAPHORE_POST, sem, 1, 0, 0);
        timed(3, SYS_SEMAPHORE_POST, sem, (u64)-1ll, 0, 0);
        step(54, (u64)cgov_sc(SYS_SEMAPHORE_DESTROY, sem, 0, 0, 0, 0, 0, 0, 0));
    }
    if (mutex_rc == 0) {
        if (step(52, (u64)cgov_sc(SYS_MUTEX_LOCK, mutex, 0, 0, 0, 0, 0, 0, 0)) == 0) {
            timed(4, SYS_MUTEX_LOCK, mutex, LOCK_TIMEOUT_US, 0, 0);
            timed(5, SYS_MUTEX_TRYLOCK, mutex, 0, 0, 0);
            if (step(53, (u64)cgov_sc(SYS_MUTEX_UNLOCK, mutex, 0, 0, 0, 0, 0, 0, 0)) == 0)
                timed(6, SYS_MUTEX_UNLOCK, mutex, 0, 0, 0);
        }
        step(55, (u64)cgov_sc(SYS_MUTEX_DESTROY, mutex, 0, 0, 0, 0, 0, 0, 0));
    }
    if (flag_rc == 0) {
        timed(7, SYS_EVENT_FLAG_TRYWAIT, flag, 1, WAIT_AND, EA(&flag_result));
        step(56, (u64)cgov_sc(SYS_EVENT_FLAG_DESTROY, flag, 0, 0, 0, 0, 0, 0, 0));
    }

    for (i = 0; i < LOOPS; i++)
        put(32 + i, first_rc[i]);
    put(57, first_failed);
    put(59, REPS);
    put(60, DONE);
    write_frame();
    return 0;
}
