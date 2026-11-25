/* PPU probe: the round trip of LV2 calls no other capture times on
 * their uncontended success path, each timed with mftb.
 *
 * Each loop makes 1 << 12 calls of one syscall, five runs a loop. No
 * call blocks: the semaphore's count covers every wait, the event flag
 * holds bit 0 for each wait in OR mode that clears nothing, and the
 * condition variable has no waiter for its signals.
 *
 *   loop
 *    0  sc 147 sys_time_get_timebase_frequency, the baseline
 *    1  sc 92  sys_semaphore_wait, the count positive, no timeout
 *    2  sc 85  sys_event_flag_wait bit 0, OR, no clear, no timeout
 *    3  sc 86  sys_event_flag_trywait bit 0, OR, no clear
 *    4  sc 108 sys_cond_signal, no waiter
 *    5  sc 109 sys_cond_signal_all, no waiter
 *    6  sc 18  sys_process_getppid
 *    7  sc 25  sys_process_get_sdk_version of the process
 *    8  sc 144 sys_time_get_timezone
 *
 *   word
 *   2L, 2L+1   loop L's fastest and slowest run, in time-base ticks
 *   32 + L     loop L's first nonzero return code, or 0
 *   48  semaphore create rc    49  event flag create rc
 *   50  mutex create rc        51  condition variable create rc
 *   52  semaphore destroy rc   53  event flag destroy rc
 *   54  condition destroy rc   55  mutex destroy rc
 *   56  the event flag's bits loop 2 read
 *   57  the SDK version loop 7 read
 *   58  the time zone loop 8 read
 *   59  runs a loop (5)
 *   60  0x5050_4C55 when the probe wrote every word
 *
 * Payload: 64 u64 words, 512 bytes.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PROCESS_GETPID               1
#define SYS_PROCESS_GETPPID              18
#define SYS_PROCESS_GET_SDK_VERSION      25
#define SYS_EVENT_FLAG_CREATE            82
#define SYS_EVENT_FLAG_DESTROY           83
#define SYS_EVENT_FLAG_WAIT              85
#define SYS_EVENT_FLAG_TRYWAIT           86
#define SYS_SEMAPHORE_CREATE             90
#define SYS_SEMAPHORE_DESTROY            91
#define SYS_SEMAPHORE_WAIT               92
#define SYS_MUTEX_CREATE                 100
#define SYS_MUTEX_DESTROY                101
#define SYS_COND_CREATE                  105
#define SYS_COND_DESTROY                 106
#define SYS_COND_SIGNAL                  108
#define SYS_COND_SIGNAL_ALL              109
#define SYS_TIME_GET_TIMEZONE            144
#define SYS_TIME_GET_TIMEBASE_FREQUENCY  147

#define FIFO               0x1
#define NOT_RECURSIVE      0x20
#define NOT_PROCESS_SHARED 0x200
#define NOT_ADAPTIVE       0x2000
#define FLAG_WAIT_OR       0x2

#define WORDS   64
#define PAYLOAD (WORDS * 8)
#define REPS    5
#define CALLS   (1u << 12)
#define LOOPS   9
#define DONE    0x50504C55ull

/* The semaphore's count: far more than the waits of loop 1's runs. */
#define SEM_COUNT 0x7FFFFFFF

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned int mutex_attr[10] __attribute__((aligned(8)));
static volatile unsigned int sem_attr[8] __attribute__((aligned(8)));
static const unsigned int flag_attr[8] __attribute__((aligned(8))) =
    { 1, 0x200, 0, 0, 0, 0x10000, 0, 0 };   /* FIFO, private, one waiter */
/* sys_cond_attribute_t: private, no flags, no key, no name. */
static const unsigned int cond_attr[6] __attribute__((aligned(8))) = { 0x200, 0, 0, 0, 0, 0 };

static volatile unsigned int id_word __attribute__((aligned(8)));
static volatile u64 flag_bits __attribute__((aligned(8)));
static volatile int sdk_version __attribute__((aligned(8)));
static volatile int time_zone __attribute__((aligned(8)));
static volatile int summer_time __attribute__((aligned(8)));

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
    u64 sem_rc, flag_rc, mutex_rc, cond_rc = ~0ull, pid;
    unsigned int sem = 0, flag = 0, mutex = 0, cond = 0;

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

    sem_rc = (u64)cgov_sc(SYS_SEMAPHORE_CREATE, EA(&id_word), EA(sem_attr), SEM_COUNT, SEM_COUNT,
                          0, 0, 0, 0);
    sem = id_word;
    flag_rc = (u64)cgov_sc(SYS_EVENT_FLAG_CREATE, EA(&id_word), EA(flag_attr), 1, 0, 0, 0, 0, 0);
    flag = id_word;
    mutex_rc = (u64)cgov_sc(SYS_MUTEX_CREATE, EA(&id_word), EA(mutex_attr), 0, 0, 0, 0, 0, 0);
    mutex = id_word;
    if (mutex_rc == 0) {
        cond_rc = (u64)cgov_sc(SYS_COND_CREATE, EA(&id_word), mutex, EA(cond_attr), 0, 0, 0, 0, 0);
        cond = id_word;
    }
    put(48, sem_rc);
    put(49, flag_rc);
    put(50, mutex_rc);
    put(51, cond_rc);

    timed(0, SYS_TIME_GET_TIMEBASE_FREQUENCY, 0, 0, 0, 0);
    if (sem_rc == 0) {
        timed(1, SYS_SEMAPHORE_WAIT, sem, 0, 0, 0);
        put(52, (u64)cgov_sc(SYS_SEMAPHORE_DESTROY, sem, 0, 0, 0, 0, 0, 0, 0));
    }
    if (flag_rc == 0) {
        timed(2, SYS_EVENT_FLAG_WAIT, flag, 1, FLAG_WAIT_OR, EA(&flag_bits));
        put(56, flag_bits);
        timed(3, SYS_EVENT_FLAG_TRYWAIT, flag, 1, FLAG_WAIT_OR, EA(&flag_bits));
        put(53, (u64)cgov_sc(SYS_EVENT_FLAG_DESTROY, flag, 0, 0, 0, 0, 0, 0, 0));
    }
    if (cond_rc == 0) {
        timed(4, SYS_COND_SIGNAL, cond, 0, 0, 0);
        timed(5, SYS_COND_SIGNAL_ALL, cond, 0, 0, 0);
        put(54, (u64)cgov_sc(SYS_COND_DESTROY, cond, 0, 0, 0, 0, 0, 0, 0));
    }
    if (mutex_rc == 0)
        put(55, (u64)cgov_sc(SYS_MUTEX_DESTROY, mutex, 0, 0, 0, 0, 0, 0, 0));
    timed(6, SYS_PROCESS_GETPPID, 0, 0, 0, 0);
    pid = (u64)cgov_sc(SYS_PROCESS_GETPID, 0, 0, 0, 0, 0, 0, 0, 0) & 0xFFFFFFFFull;
    timed(7, SYS_PROCESS_GET_SDK_VERSION, pid, EA(&sdk_version), 0, 0);
    put(57, (u64)(unsigned int)sdk_version);
    timed(8, SYS_TIME_GET_TIMEZONE, EA(&time_zone), EA(&summer_time), 0, 0);
    put(58, (u64)(unsigned int)time_zone);

    for (i = 0; i < LOOPS; i++)
        put(32 + i, first_rc[i]);
    put(59, REPS);
    put(60, DONE);
    write_frame();
    return 0;
}
