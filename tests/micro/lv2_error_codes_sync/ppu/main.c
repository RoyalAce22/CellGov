/* PPU probe: the CellError codes the mutex, semaphore and memory calls
 * give, one call apiece.
 *
 * Every case is one raw `sc`; the payload holds its return code, the
 * full r3 as the kernel left it. The probe makes one mutex and one
 * semaphore, removes both, and touches nothing outside its process.
 * No call is repeated to use up a kernel limit.
 *
 * Cases, in payload order (u64 r3 each, 8 bytes):
 *    0  sc 100 mutex create, protocol 0                    (EINVAL)
 *    1  sc 101 mutex destroy, id 0                         (ESRCH)
 *    2  sc 90 semaphore create, attribute NULL             (EFAULT)
 *    3  sc 102 mutex lock, a mutex this thread holds       (EDEADLK)
 *    4  sc 103 mutex trylock, the same                     (EDEADLK)
 *    5  sc 101 mutex destroy, the same while held          (EBUSY)
 *    6  sc 104 mutex unlock, once released                 (EPERM)
 *    7  sc 92 semaphore wait, count 0, 1 us timeout        (ETIMEDOUT)
 *    8  sc 93 semaphore trywait, count 0                   (EBUSY)
 *    9  sc 94 semaphore post 2 into a max-1 semaphore      (EBUSY)
 *   10  sc 348 allocate 256 MiB                            (ENOMEM)
 *   11  sc 348 allocate 0x10001 bytes of 64 KiB pages      (EALIGN)
 *
 * Setup and cleanup answers, after the cases (u64 r3 each):
 *   12  mutex create      13  mutex lock      14  mutex unlock
 *   15  mutex destroy     16  semaphore create
 *   17  semaphore destroy
 *
 * Payload: 18 u64 words, 144 bytes.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_SEMAPHORE_CREATE   90
#define SYS_SEMAPHORE_DESTROY  91
#define SYS_SEMAPHORE_WAIT     92
#define SYS_SEMAPHORE_TRYWAIT  93
#define SYS_SEMAPHORE_POST     94
#define SYS_MUTEX_CREATE       100
#define SYS_MUTEX_DESTROY      101
#define SYS_MUTEX_LOCK         102
#define SYS_MUTEX_TRYLOCK      103
#define SYS_MUTEX_UNLOCK       104
#define SYS_MEMORY_ALLOCATE    348

#define FIFO               0x1
#define NOT_RECURSIVE      0x20
#define NOT_PROCESS_SHARED 0x200
#define NOT_ADAPTIVE       0x2000

#define PAGE_64K 0x200
#define PAGE_1M  0x400

#define WORDS 18

static volatile u64 result[WORDS] __attribute__((aligned(128)));
static volatile unsigned int mutex_attr[10] __attribute__((aligned(8)));
static volatile unsigned int bad_mutex_attr[10] __attribute__((aligned(8)));
static volatile unsigned int sem_attr[8] __attribute__((aligned(8)));
static volatile unsigned int id_word __attribute__((aligned(8)));
static volatile unsigned int addr_word __attribute__((aligned(8)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((unsigned long)(const void *)(p))

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

static u64 sc(u64 num, u64 a, u64 b, u64 c, u64 d)
{
    return (u64)cgov_sc(num, a, b, c, d, 0, 0, 0, 0);
}

static void mutex_block(volatile unsigned int *p, unsigned int protocol)
{
    unsigned int i;
    for (i = 0; i < 10; i++)
        p[i] = 0;
    p[0] = protocol;
    p[1] = NOT_RECURSIVE;
    p[2] = NOT_PROCESS_SHARED;
    p[3] = NOT_ADAPTIVE;
}

int main(void)
{
    unsigned int mutex, sem, i;

    for (i = 0; i < WORDS; i++)
        result[i] = 0xEEEEEEEEEEEEEEEEULL;
    mutex_block(mutex_attr, FIFO);
    mutex_block(bad_mutex_attr, 0);
    for (i = 0; i < 8; i++)
        sem_attr[i] = 0;
    sem_attr[0] = FIFO;
    sem_attr[1] = NOT_PROCESS_SHARED;

    /* Arguments with no object behind them. A create that succeeds
     * anyway is destroyed at once. */
    result[0] = sc(SYS_MUTEX_CREATE, EA(&id_word), EA(bad_mutex_attr), 0, 0);
    if (result[0] == 0)
        (void)sc(SYS_MUTEX_DESTROY, id_word, 0, 0, 0);
    result[1] = sc(SYS_MUTEX_DESTROY, 0, 0, 0, 0);
    result[2] = sc(SYS_SEMAPHORE_CREATE, EA(&id_word), 0, 0, 1);
    if (result[2] == 0)
        (void)sc(SYS_SEMAPHORE_DESTROY, id_word, 0, 0, 0);

    /* A mutex this thread holds. */
    result[12] = sc(SYS_MUTEX_CREATE, EA(&id_word), EA(mutex_attr), 0, 0);
    mutex = id_word;
    if (result[12] == 0) {
        result[13] = sc(SYS_MUTEX_LOCK, mutex, 0, 0, 0);
        result[3] = sc(SYS_MUTEX_LOCK, mutex, 0, 0, 0);
        result[4] = sc(SYS_MUTEX_TRYLOCK, mutex, 0, 0, 0);
        result[5] = sc(SYS_MUTEX_DESTROY, mutex, 0, 0, 0);
        result[14] = sc(SYS_MUTEX_UNLOCK, mutex, 0, 0, 0);
        result[6] = sc(SYS_MUTEX_UNLOCK, mutex, 0, 0, 0);
        result[15] = sc(SYS_MUTEX_DESTROY, mutex, 0, 0, 0);
    }

    /* A semaphore with count 0 and max 1. */
    result[16] = sc(SYS_SEMAPHORE_CREATE, EA(&id_word), EA(sem_attr), 0, 1);
    sem = id_word;
    if (result[16] == 0) {
        result[7] = sc(SYS_SEMAPHORE_WAIT, sem, 1, 0, 0);
        result[8] = sc(SYS_SEMAPHORE_TRYWAIT, sem, 0, 0, 0);
        result[9] = sc(SYS_SEMAPHORE_POST, sem, 2, 0, 0);
        result[17] = sc(SYS_SEMAPHORE_DESTROY, sem, 0, 0, 0);
    }

    /* Memory. Neither call can succeed; a success is freed at process
     * exit and shows in the payload. */
    result[10] = sc(SYS_MEMORY_ALLOCATE, 0x10000000, PAGE_1M, EA(&addr_word), 0);
    result[11] = sc(SYS_MEMORY_ALLOCATE, 0x10001, PAGE_64K, EA(&addr_word), 0);

    write_frame();
    return 0;
}
