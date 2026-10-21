/* PPU probe: the round trip of one LV2 call of each class, on the
 * uncontended path, each timed with mftb.
 *
 * Each loop makes 1 << 12 calls of one syscall, five runs a loop. The
 * semaphore has room for every post, and the mutex is recursive, so a
 * loop of posts and then a loop of waits, or a loop of locks and then a
 * loop of unlocks, times each call alone. No call blocks: the semaphore
 * waits are trywaits on a positive count, the mutex has no other owner
 * and the event queue is empty for its tryreceive.
 *
 *   loop
 *    0  sc 147 sys_time_get_timebase_frequency
 *    1  sc 145 sys_time_get_current_time
 *    2  sc 352 sys_memory_get_user_memory_size
 *    3  sc 43  sys_ppu_thread_yield, no other thread runnable
 *    4  sc 114 sys_semaphore_get_value
 *    5  sc 94  sys_semaphore_post 1
 *    6  sc 93  sys_semaphore_trywait, the count loop 5 left
 *    7  sc 139 sys_event_flag_get
 *    8  sc 87  sys_event_flag_set bit 0
 *    9  sc 118 sys_event_flag_clear, every bit
 *   10  sc 102 sys_mutex_lock, recursive, no timeout
 *   11  sc 104 sys_mutex_unlock, the locks loop 10 took
 *   12  sc 103 sys_mutex_trylock, recursive
 *   13  sc 104 sys_mutex_unlock, the locks loop 12 took
 *   14  sc 131 sys_event_queue_tryreceive, the queue empty
 *
 *   word
 *   2L, 2L+1   loop L's fastest and slowest run, in time-base ticks
 *   32 + L     loop L's first nonzero return code, or 0
 *   48  semaphore create rc    49  event flag create rc
 *   50  mutex create rc        51  event queue create rc
 *   52  semaphore destroy rc   53  event flag destroy rc
 *   54  mutex destroy rc       55  event queue destroy rc
 *   56  the time-base frequency loop 0 read
 *   57  the semaphore value loop 4 read
 *   58  the event count loop 14 read
 *   59  runs a loop (5)
 *   60  0x5050_4C43 when the probe wrote every word
 *
 * Payload: 64 u64 words, 512 bytes.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PPU_THREAD_YIELD             43
#define SYS_EVENT_FLAG_CREATE            82
#define SYS_EVENT_FLAG_DESTROY           83
#define SYS_EVENT_FLAG_SET               87
#define SYS_SEMAPHORE_CREATE             90
#define SYS_SEMAPHORE_DESTROY            91
#define SYS_SEMAPHORE_TRYWAIT            93
#define SYS_SEMAPHORE_POST               94
#define SYS_MUTEX_CREATE                 100
#define SYS_MUTEX_DESTROY                101
#define SYS_MUTEX_LOCK                   102
#define SYS_MUTEX_TRYLOCK                103
#define SYS_MUTEX_UNLOCK                 104
#define SYS_SEMAPHORE_GET_VALUE          114
#define SYS_EVENT_FLAG_CLEAR             118
#define SYS_EVENT_QUEUE_CREATE           128
#define SYS_EVENT_QUEUE_DESTROY          129
#define SYS_EVENT_QUEUE_TRYRECEIVE       131
#define SYS_EVENT_FLAG_GET               139
#define SYS_TIME_GET_CURRENT_TIME        145
#define SYS_TIME_GET_TIMEBASE_FREQUENCY  147
#define SYS_MEMORY_GET_USER_MEMORY_SIZE  352

#define FIFO               0x1
#define RECURSIVE          0x10
#define NOT_PROCESS_SHARED 0x200
#define NOT_ADAPTIVE       0x2000

#define WORDS   64
#define PAYLOAD (WORDS * 8)
#define REPS    5
#define CALLS   (1u << 12)
#define LOOPS   15
#define DONE    0x50504C43ull

/* Room for every post of loop 5's five runs. */
#define SEM_MAX 0x7FFFFFFF

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned int mutex_attr[10] __attribute__((aligned(8)));
static volatile unsigned int sem_attr[8] __attribute__((aligned(8)));
static const unsigned int flag_attr[8] __attribute__((aligned(8))) =
    { 1, 0x200, 0, 0, 0, 0x10000, 0, 0 };   /* FIFO, private, one waiter */
/* sys_event_queue_attribute_t: FIFO, a PPU queue, no name. */
static const unsigned int queue_attr[4] __attribute__((aligned(8))) = { 1, 1, 0, 0 };

static volatile unsigned int id_word __attribute__((aligned(8)));
static volatile u64 sec_word __attribute__((aligned(8)));
static volatile u64 nsec_word __attribute__((aligned(8)));
static volatile u64 mem_info[2] __attribute__((aligned(8)));
static volatile int sem_value __attribute__((aligned(8)));
static volatile u64 flag_value __attribute__((aligned(8)));
static volatile u64 events[4] __attribute__((aligned(8)));
static volatile int event_count __attribute__((aligned(8)));

static u64 first_rc[LOOPS];

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
 * Keeps the first nonzero return code in first_rc[loop]. */
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
    u64 sem_rc, flag_rc, mutex_rc, queue_rc;
    unsigned int sem = 0, flag = 0, mutex = 0, queue = 0;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < 10; i++)
        mutex_attr[i] = 0;
    mutex_attr[0] = FIFO;
    mutex_attr[1] = RECURSIVE;
    mutex_attr[2] = NOT_PROCESS_SHARED;
    mutex_attr[3] = NOT_ADAPTIVE;
    for (i = 0; i < 8; i++)
        sem_attr[i] = 0;
    sem_attr[0] = FIFO;
    sem_attr[1] = NOT_PROCESS_SHARED;

    sem_rc = (u64)cgov_sc(SYS_SEMAPHORE_CREATE, EA(&id_word), EA(sem_attr), 0, SEM_MAX, 0, 0, 0, 0);
    sem = id_word;
    flag_rc = (u64)cgov_sc(SYS_EVENT_FLAG_CREATE, EA(&id_word), EA(flag_attr), 0, 0, 0, 0, 0, 0);
    flag = id_word;
    mutex_rc = (u64)cgov_sc(SYS_MUTEX_CREATE, EA(&id_word), EA(mutex_attr), 0, 0, 0, 0, 0, 0);
    mutex = id_word;
    queue_rc = (u64)cgov_sc(SYS_EVENT_QUEUE_CREATE, EA(&id_word), EA(queue_attr), 0, 16, 0, 0, 0, 0);
    queue = id_word;
    put(48, sem_rc);
    put(49, flag_rc);
    put(50, mutex_rc);
    put(51, queue_rc);

    timed(0, SYS_TIME_GET_TIMEBASE_FREQUENCY, 0, 0, 0, 0);
    put(56, (u64)cgov_sc(SYS_TIME_GET_TIMEBASE_FREQUENCY, 0, 0, 0, 0, 0, 0, 0, 0));
    timed(1, SYS_TIME_GET_CURRENT_TIME, EA(&sec_word), EA(&nsec_word), 0, 0);
    timed(2, SYS_MEMORY_GET_USER_MEMORY_SIZE, EA(mem_info), 0, 0, 0);
    timed(3, SYS_PPU_THREAD_YIELD, 0, 0, 0, 0);

    if (sem_rc == 0) {
        timed(4, SYS_SEMAPHORE_GET_VALUE, sem, EA(&sem_value), 0, 0);
        put(57, (u64)(unsigned int)sem_value);
        timed(5, SYS_SEMAPHORE_POST, sem, 1, 0, 0);
        timed(6, SYS_SEMAPHORE_TRYWAIT, sem, 0, 0, 0);
        put(52, (u64)cgov_sc(SYS_SEMAPHORE_DESTROY, sem, 0, 0, 0, 0, 0, 0, 0));
    }
    if (flag_rc == 0) {
        timed(7, SYS_EVENT_FLAG_GET, flag, EA(&flag_value), 0, 0);
        timed(8, SYS_EVENT_FLAG_SET, flag, 1, 0, 0);
        timed(9, SYS_EVENT_FLAG_CLEAR, flag, 0, 0, 0);
        put(53, (u64)cgov_sc(SYS_EVENT_FLAG_DESTROY, flag, 0, 0, 0, 0, 0, 0, 0));
    }
    if (mutex_rc == 0) {
        timed(10, SYS_MUTEX_LOCK, mutex, 0, 0, 0);
        timed(11, SYS_MUTEX_UNLOCK, mutex, 0, 0, 0);
        timed(12, SYS_MUTEX_TRYLOCK, mutex, 0, 0, 0);
        timed(13, SYS_MUTEX_UNLOCK, mutex, 0, 0, 0);
        put(54, (u64)cgov_sc(SYS_MUTEX_DESTROY, mutex, 0, 0, 0, 0, 0, 0, 0));
    }
    if (queue_rc == 0) {
        event_count = -1;
        timed(14, SYS_EVENT_QUEUE_TRYRECEIVE, queue, EA(events), 1, EA(&event_count));
        put(58, (u64)(unsigned int)event_count);
        put(55, (u64)cgov_sc(SYS_EVENT_QUEUE_DESTROY, queue, 0, 0, 0, 0, 0, 0, 0));
    }

    for (i = 0; i < LOOPS; i++)
        put(32 + i, first_rc[i]);
    put(59, REPS);
    put(60, DONE);
    write_frame();
    return 0;
}
