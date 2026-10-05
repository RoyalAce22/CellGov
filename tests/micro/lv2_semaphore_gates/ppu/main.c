/* PPU probe: the gates of sys_semaphore_create (90), get_value (114)
 * and post (94), and the four sync creates given an attribute block
 * whose name bytes lie on an unmapped page.
 *
 * Steps (the payload records every return code):
 *   A. sc 90(NULL, attr, 5, 1); sc 90(&id, NULL, -1, 0);
 *      sc 114(0xDEADBEEF, NULL).
 *   B. A semaphore with initial 0 and max 1: sc 94(id, 0), then
 *      sc 114 for the value; sc 94(id, -1), then sc 114 again; destroy.
 *   C. sc 348 allocates 1 MiB of 1 MiB pages. sc 351 reads the page
 *      attribute of the first byte and of the byte just past the end,
 *      so the payload shows whether the next page is unmapped. A
 *      helper mutex is created for the cond create.
 *   D. Each of sem (90), mutex (100), event flag (82) and cond (105)
 *      gets a valid attribute block placed so that its last 8 bytes
 *      (the name) start at the end of the allocation. A create that
 *      succeeds is destroyed at once.
 *   E. The same four blocks placed 64 bytes before the end (control).
 *   F. The helper mutex is destroyed and the allocation freed.
 *
 * Every attribute word is written with its own 32-bit volatile store,
 * and the probe never writes a block's name bytes.
 *
 * Payload layout (26 big-endian u32 words, 104 bytes):
 *   0  step that failed (0 = none; 1 = semaphore create in B,
 *      2 = allocate, 3 = helper mutex create)
 *   1  that step's return code
 *   2  sc 90(NULL, attr, 5, 1)
 *   3  sc 90(&id, NULL, -1, 0)
 *   4  sc 114(0xDEADBEEF, NULL)
 *   5  B: semaphore create
 *   6  B: sc 94(id, 0)
 *   7  B: sc 114 return code after it
 *   8  B: the value it read
 *   9  B: sc 94(id, -1)
 *   10 B: sc 114 return code after it
 *   11 B: the value it read
 *   12 B: semaphore destroy
 *   13 C: sc 348
 *   14 C: sc 351 on the first byte
 *   15 C: sc 351 on the byte past the end
 *   16 D: sem create across the end
 *   17 D: mutex create across the end
 *   18 D: event flag create across the end
 *   19 D: cond create across the end
 *   20 E: sem create inside
 *   21 E: mutex create inside
 *   22 E: event flag create inside
 *   23 E: cond create inside
 *   24 F: helper mutex destroy
 *   25 F: sc 349
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_EVENT_FLAG_CREATE    82
#define SYS_EVENT_FLAG_DESTROY   83
#define SYS_SEMAPHORE_CREATE     90
#define SYS_SEMAPHORE_DESTROY    91
#define SYS_SEMAPHORE_POST       94
#define SYS_MUTEX_CREATE         100
#define SYS_MUTEX_DESTROY        101
#define SYS_COND_CREATE          105
#define SYS_COND_DESTROY         106
#define SYS_SEMAPHORE_GET_VALUE  114
#define SYS_MEMORY_ALLOCATE      348
#define SYS_MEMORY_FREE          349
#define SYS_MEMORY_GET_PAGE_ATTRIBUTE 351

#define FIFO               0x1
#define NOT_RECURSIVE      0x20
#define NOT_PROCESS_SHARED 0x200
#define NOT_ADAPTIVE       0x2000
#define WAITER_SINGLE      0x10000
#define PAGE_SIZE_1M       0x400

#define ALLOC_SIZE 0x100000u

#define WORDS 26

static unsigned int result[WORDS] __attribute__((aligned(128)));
static unsigned int ids[4] __attribute__((aligned(8)));
static volatile unsigned int value_word __attribute__((aligned(8)));
static unsigned int alloc_addr __attribute__((aligned(8)));
static unsigned long long page_attr[4] __attribute__((aligned(8)));

/* The semaphore attribute block of steps A and B: FIFO, not
 * process-shared, key 0, flags 0, name zero. 32 bytes. */
static volatile unsigned int sem_attr[8] __attribute__((aligned(8)));
/* The mutex attribute block of the helper mutex. 40 bytes. */
static volatile unsigned int mutex_attr[10] __attribute__((aligned(8)));

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

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    result[0] = step;
    result[1] = (unsigned int)rc;
    write_result();
    return (int)step;
}

/* Zero the first `words - 2` u32 words at `p`. The last two words are
 * the 8-byte name, which no create checks; in step D they lie on the
 * unmapped page, so the probe never writes them. */
static void put_block(volatile unsigned int *p, unsigned int words)
{
    unsigned int i;
    for (i = 0; i + 2 < words; i++)
        p[i] = 0;
}

static void sem_block(volatile unsigned int *p)
{
    put_block(p, 8);
    p[0] = FIFO;                 /* protocol@0 */
    p[1] = NOT_PROCESS_SHARED;   /* pshared@4 */
}

static void mutex_block(volatile unsigned int *p)
{
    put_block(p, 10);
    p[0] = FIFO;                 /* protocol@0 */
    p[1] = NOT_RECURSIVE;        /* recursive@4 */
    p[2] = NOT_PROCESS_SHARED;   /* pshared@8 */
    p[3] = NOT_ADAPTIVE;         /* adaptive@12 */
}

static void flag_block(volatile unsigned int *p)
{
    put_block(p, 8);
    p[0] = FIFO;                 /* protocol@0 */
    p[1] = NOT_PROCESS_SHARED;   /* pshared@4 */
    p[5] = WAITER_SINGLE;        /* type@20 */
}

static void cond_block(volatile unsigned int *p)
{
    put_block(p, 6);
    p[0] = NOT_PROCESS_SHARED;   /* pshared@0 */
}

#define EA(p) ((unsigned long)(const void *)(p))

/* Run the four creates with blocks whose name starts at `name_at`,
 * recording the return codes at result[first..first+3]. */
static void four_creates(unsigned int name_at, unsigned int helper_mutex, unsigned int first)
{
    volatile unsigned int *blk;
    s32 rc;

    blk = (volatile unsigned int *)(unsigned long)(name_at - 24);
    sem_block(blk);
    rc = syscall4_s32(SYS_SEMAPHORE_CREATE, EA(&ids[0]), EA(blk), 0, 1);
    result[first] = (unsigned int)rc;
    if (rc == 0)
        (void)syscall1_s32(SYS_SEMAPHORE_DESTROY, ids[0]);

    blk = (volatile unsigned int *)(unsigned long)(name_at - 32);
    mutex_block(blk);
    rc = syscall2_s32(SYS_MUTEX_CREATE, EA(&ids[1]), EA(blk));
    result[first + 1] = (unsigned int)rc;
    if (rc == 0)
        (void)syscall1_s32(SYS_MUTEX_DESTROY, ids[1]);

    blk = (volatile unsigned int *)(unsigned long)(name_at - 24);
    flag_block(blk);
    rc = syscall3_s32(SYS_EVENT_FLAG_CREATE, EA(&ids[2]), EA(blk), 0);
    result[first + 2] = (unsigned int)rc;
    if (rc == 0)
        (void)syscall1_s32(SYS_EVENT_FLAG_DESTROY, ids[2]);

    blk = (volatile unsigned int *)(unsigned long)(name_at - 16);
    cond_block(blk);
    rc = syscall3_s32(SYS_COND_CREATE, EA(&ids[3]), helper_mutex, EA(blk));
    result[first + 3] = (unsigned int)rc;
    if (rc == 0)
        (void)syscall1_s32(SYS_COND_DESTROY, ids[3]);
}

int main(void)
{
    s32 rc;
    unsigned int sem_id = 0, helper = 0, end;

    memset(result, 0, sizeof(result));
    sem_block(sem_attr);
    mutex_block(mutex_attr);

    /* A. */
    result[2] = (unsigned int)syscall4_s32(SYS_SEMAPHORE_CREATE, 0, EA(sem_attr), 5, 1);
    result[3] = (unsigned int)syscall4_s32(SYS_SEMAPHORE_CREATE, EA(&ids[0]), 0,
                                           (unsigned long)(long)-1, 0);
    result[4] = (unsigned int)syscall2_s32(SYS_SEMAPHORE_GET_VALUE, 0xDEADBEEFu, 0);

    /* B. */
    rc = syscall4_s32(SYS_SEMAPHORE_CREATE, EA(&sem_id), EA(sem_attr), 0, 1);
    result[5] = (unsigned int)rc;
    if (rc != 0)
        return fail(1, rc);
    result[6] = (unsigned int)syscall2_s32(SYS_SEMAPHORE_POST, sem_id, 0);
    value_word = 0xFFFFFFFFu;
    result[7] = (unsigned int)syscall2_s32(SYS_SEMAPHORE_GET_VALUE, sem_id, EA(&value_word));
    result[8] = value_word;
    result[9] = (unsigned int)syscall2_s32(SYS_SEMAPHORE_POST, sem_id,
                                           (unsigned long)(long)-1);
    value_word = 0xFFFFFFFFu;
    result[10] = (unsigned int)syscall2_s32(SYS_SEMAPHORE_GET_VALUE, sem_id, EA(&value_word));
    result[11] = value_word;
    result[12] = (unsigned int)syscall1_s32(SYS_SEMAPHORE_DESTROY, sem_id);

    /* C. */
    rc = syscall3_s32(SYS_MEMORY_ALLOCATE, ALLOC_SIZE, PAGE_SIZE_1M, EA(&alloc_addr));
    result[13] = (unsigned int)rc;
    if (rc != 0)
        return fail(2, rc);
    end = alloc_addr + ALLOC_SIZE;
    result[14] = (unsigned int)syscall2_s32(SYS_MEMORY_GET_PAGE_ATTRIBUTE, alloc_addr,
                                            EA(page_attr));
    result[15] = (unsigned int)syscall2_s32(SYS_MEMORY_GET_PAGE_ATTRIBUTE, end,
                                            EA(page_attr));
    rc = syscall2_s32(SYS_MUTEX_CREATE, EA(&helper), EA(mutex_attr));
    if (rc != 0)
        return fail(3, rc);

    /* D, then E. */
    four_creates(end, helper, 16);
    four_creates(end - 64, helper, 20);

    /* F. */
    result[24] = (unsigned int)syscall1_s32(SYS_MUTEX_DESTROY, helper);
    result[25] = (unsigned int)syscall1_s32(SYS_MEMORY_FREE, alloc_addr);

    write_result();
    return 0;
}
