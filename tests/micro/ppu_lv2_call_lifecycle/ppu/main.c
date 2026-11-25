/* PPU probe: the round trip of creating and destroying each LV2
 * synchronization object class, each timed with mftb.
 *
 * Each run creates 128 objects of one class, one call each, then
 * destroys them in the order it created them. Five runs a class. The
 * creates and the destroys are timed as separate loops. Two loops of
 * sys_time_get_timebase_frequency, one of each loop's shape, are the
 * baseline. The condition variables share one mutex, created before
 * their runs and destroyed after them.
 *
 *   loop
 *    0  sc 147 sys_time_get_timebase_frequency, the create loop's shape
 *    1  sc 147 sys_time_get_timebase_frequency, the destroy loop's shape
 *    2  sc 95  _sys_lwmutex_create, FIFO, no name
 *    3  sc 96  _sys_lwmutex_destroy
 *    4  sc 100 sys_mutex_create, FIFO and nonrecursive
 *    5  sc 101 sys_mutex_destroy
 *    6  sc 90  sys_semaphore_create, count 1 of 1
 *    7  sc 91  sys_semaphore_destroy
 *    8  sc 82  sys_event_flag_create, bits 0
 *    9  sc 83  sys_event_flag_destroy
 *   10  sc 105 sys_cond_create on the shared mutex
 *   11  sc 106 sys_cond_destroy
 *
 *   word
 *   2L, 2L+1   loop L's fastest and slowest run, in time-base ticks
 *   32 + L     loop L's first nonzero return code, or 0
 *   48  the shared mutex's create rc   49  its destroy rc
 *   58  objects a run (128)
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

#define SYS_EVENT_FLAG_CREATE            82
#define SYS_EVENT_FLAG_DESTROY           83
#define SYS_SEMAPHORE_CREATE             90
#define SYS_SEMAPHORE_DESTROY            91
#define SYS_LWMUTEX_CREATE               95
#define SYS_LWMUTEX_DESTROY              96
#define SYS_MUTEX_CREATE                 100
#define SYS_MUTEX_DESTROY                101
#define SYS_COND_CREATE                  105
#define SYS_COND_DESTROY                 106
#define SYS_TIME_GET_TIMEBASE_FREQUENCY  147

#define FIFO 0x1

#define WORDS   64
#define PAYLOAD (WORDS * 8)
#define REPS    5
#define OBJECTS 128
#define LOOPS   12
#define DONE    0x50504C55ull

/* sys_lwmutex_t: 24 bytes, kept 32 apart. */
#define LW_STRIDE 32

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
/* sys_mutex_attribute_t: FIFO, nonrecursive, private and nonadaptive. */
static const unsigned int mutex_attr[10] __attribute__((aligned(8))) =
    { FIFO, 0x20, 0x200, 0x2000, 0, 0, 0, 0, 0, 0 };
/* sys_semaphore_attribute_t: FIFO, private. */
static const unsigned int sem_attr[8] __attribute__((aligned(8))) = { FIFO, 0x200, 0, 0, 0, 0, 0, 0 };
/* sys_event_flag_attribute_t: FIFO, private, one waiter. */
static const unsigned int flag_attr[8] __attribute__((aligned(8))) =
    { FIFO, 0x200, 0, 0, 0, 0x10000, 0, 0 };
/* sys_cond_attribute_t: private, no flags, no key, no name. */
static const unsigned int cond_attr[6] __attribute__((aligned(8))) = { 0x200, 0, 0, 0, 0, 0 };

static unsigned char lw_control[OBJECTS * LW_STRIDE] __attribute__((aligned(8)));
static volatile unsigned int ids[OBJECTS] __attribute__((aligned(8)));
static volatile unsigned int id_word __attribute__((aligned(8)));

static u64 first_rc[LOOPS];
static u64 lo[LOOPS], hi[LOOPS];

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

static void keep(unsigned loop, u64 t, u64 bad)
{
    if (bad && !first_rc[loop])
        first_rc[loop] = bad;
    if (t < lo[loop])
        lo[loop] = t;
    if (t > hi[loop])
        hi[loop] = t;
}

/* OBJECTS calls of syscall `num`, the i-th with &ids[i] in r3, `b` in
 * r4, c + i * c_stride in r5 and `d` in r6. */
static void __attribute__((noinline)) creates(unsigned loop, u64 num, u64 b, u64 c,
                                              u64 c_stride, u64 d)
{
    u64 i, rc, bad = 0;
    u64 t0 = timebase();
    for (i = 0; i < OBJECTS; i++) {
        rc = (u64)cgov_sc(num, EA(&ids[i]), b, c + i * c_stride, d, 0, 0, 0, 0);
        bad = bad ? bad : (rc & 0xFFFFFFFFull);
    }
    keep(loop, timebase() - t0, bad);
}

/* OBJECTS calls of syscall `num`, the i-th with ids[i] in r3. */
static void __attribute__((noinline)) destroys(unsigned loop, u64 num)
{
    u64 i, rc, bad = 0;
    u64 t0 = timebase();
    for (i = 0; i < OBJECTS; i++) {
        rc = (u64)cgov_sc(num, ids[i], 0, 0, 0, 0, 0, 0, 0);
        bad = bad ? bad : (rc & 0xFFFFFFFFull);
    }
    keep(loop, timebase() - t0, bad);
}

int main(void)
{
    unsigned i, r;
    u64 mutex_rc;
    unsigned int mutex = 0;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < LOOPS; i++) {
        lo[i] = ~0ull;
        hi[i] = 0;
    }

    mutex_rc = (u64)cgov_sc(SYS_MUTEX_CREATE, EA(&id_word), EA(mutex_attr), 0, 0, 0, 0, 0, 0);
    mutex = id_word;
    put(48, mutex_rc);

    for (r = 0; r < REPS; r++) {
        creates(0, SYS_TIME_GET_TIMEBASE_FREQUENCY, 0, 0, 0, 0);
        destroys(1, SYS_TIME_GET_TIMEBASE_FREQUENCY);
    }
    for (r = 0; r < REPS; r++) {
        creates(2, SYS_LWMUTEX_CREATE, FIFO, EA(lw_control), LW_STRIDE, 0);
        destroys(3, SYS_LWMUTEX_DESTROY);
    }
    for (r = 0; r < REPS; r++) {
        creates(4, SYS_MUTEX_CREATE, EA(mutex_attr), 0, 0, 0);
        destroys(5, SYS_MUTEX_DESTROY);
    }
    for (r = 0; r < REPS; r++) {
        creates(6, SYS_SEMAPHORE_CREATE, EA(sem_attr), 1, 0, 1);
        destroys(7, SYS_SEMAPHORE_DESTROY);
    }
    for (r = 0; r < REPS; r++) {
        creates(8, SYS_EVENT_FLAG_CREATE, EA(flag_attr), 0, 0, 0);
        destroys(9, SYS_EVENT_FLAG_DESTROY);
    }
    if (mutex_rc == 0) {
        for (r = 0; r < REPS; r++) {
            creates(10, SYS_COND_CREATE, mutex, EA(cond_attr), 0, 0);
            destroys(11, SYS_COND_DESTROY);
        }
        put(49, (u64)cgov_sc(SYS_MUTEX_DESTROY, mutex, 0, 0, 0, 0, 0, 0, 0));
    }

    for (i = 0; i < LOOPS; i++) {
        put(2 * i, lo[i]);
        put(2 * i + 1, hi[i]);
        put(32 + i, first_rc[i]);
    }
    put(58, OBJECTS);
    put(59, REPS);
    put(60, DONE);
    write_frame();
    return 0;
}
