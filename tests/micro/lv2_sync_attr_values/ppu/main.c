/* PPU probe: the enumerants and attribute-block layouts the sync creates
 * read, swept one field at a time.
 *
 * Every case starts from a valid block -- FIFO, not recursive, not
 * process-shared, not adaptive, waiter single, key 0, flags 0, name 0
 * -- and changes the fields it names. A process-shared case also sets a
 * key of its own (KEY_BASE + its case number) and flags 1. The id word
 * is filled with 0xFFFFFFFF before each create; a create that succeeds
 * is destroyed at once.
 *
 * Cases, in payload order:
 *   0..8    semaphore (90), protocol 0..8
 *   9..14   semaphore, pshared 0, 0x80, 0x100, 0x200, 0x300, 0x400
 *   15..23  mutex (100), protocol 0..8
 *   24..29  mutex, recursive 0, 0x8, 0x10, 0x20, 0x30, 0x40
 *   30..35  mutex, pshared as for the semaphore
 *   36..44  event flag (82), protocol 0..8
 *   45..50  event flag, type 0, 0x8000, 0x10000, 0x20000, 0x30000,
 *           0x40000
 *   51..56  event flag, pshared as for the semaphore
 *   57..62  cond (105), pshared as for the semaphore, on a helper mutex
 *   63..71  event port (134), type 0..8, name 0
 *   72      event port, type 3, name KEY_BASE + 72
 *   73      mutex, protocol@0 = 0x20 and recursive@4 = 0x1 (swapped)
 *   74      mutex, pshared@8 = 0x2000 and adaptive@0xC = 0x200 (swapped)
 *   75      event flag, flags@0x10 = 0x10000 and type@0x14 = 0 (swapped)
 *   76..79  semaphore, mutex, event flag, cond: a valid block whose last
 *           byte is the last byte of a 1 MiB allocation
 *   80..83  the same four blocks moved 4 bytes later, so that their last
 *           4 bytes lie past the allocation
 *
 * Payload layout (big-endian u32 words):
 *   +0   failed step (0 = none; 1 = allocate, 2 = helper mutex), its
 *        return code, sc 351 on the byte past the allocation, 0
 *   +16 + 16k, case k: create return code, 1 when the call changed the
 *        id word, the destroy return code (0xFFFFFFFF when the create
 *        failed), 0
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
#define SYS_MUTEX_CREATE         100
#define SYS_MUTEX_DESTROY        101
#define SYS_COND_CREATE          105
#define SYS_COND_DESTROY         106
#define SYS_EVENT_PORT_CREATE    134
#define SYS_EVENT_PORT_DESTROY   135
#define SYS_MEMORY_ALLOCATE      348
#define SYS_MEMORY_FREE          349
#define SYS_MEMORY_GET_PAGE_ATTRIBUTE 351

#define FIFO               0x1
#define NOT_RECURSIVE      0x20
#define PROCESS_SHARED     0x100
#define NOT_PROCESS_SHARED 0x200
#define NOT_ADAPTIVE       0x2000
#define WAITER_SINGLE      0x10000
#define PAGE_SIZE_1M       0x400

#define KEY_BASE   0x4347000000000000ull
#define ALLOC_SIZE 0x100000u
#define NOT_RUN    0xFFFFFFFFu
#define CASES      84

static volatile unsigned int result[4 + 4 * CASES] __attribute__((aligned(128)));
static volatile unsigned int id_word __attribute__((aligned(8)));
static unsigned int alloc_addr __attribute__((aligned(8)));
static unsigned long long page_attr[4] __attribute__((aligned(8)));

/* Room for the largest block, the mutex's 40 bytes. */
static volatile unsigned int block[10] __attribute__((aligned(8)));

static const unsigned int PSHARED[6] = { 0, 0x80, 0x100, 0x200, 0x300, 0x400 };
static const unsigned int RECURSIVE[6] = { 0, 0x8, 0x10, 0x20, 0x30, 0x40 };
static const unsigned int TYPE[6] = { 0, 0x8000, 0x10000, 0x20000, 0x30000, 0x40000 };

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static unsigned int next_case;
static unsigned int helper;

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
    sysTtyWrite(0, (const void *)result, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)result, len);
}

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    result[0] = step;
    result[1] = (unsigned int)rc;
    write_result();
    return (int)step;
}

#define EA(p) ((unsigned long)(const void *)(p))

/* Write `words` u32 words of `src` to `dst`, one volatile store each. */
static void put_words(volatile unsigned int *dst, const unsigned int *src, unsigned int words)
{
    unsigned int i;
    for (i = 0; i < words; i++)
        dst[i] = src[i];
}

/* Record one create and destroy its object when it succeeded. */
static void record(s32 rc, unsigned int destroy_sc)
{
    unsigned int at = 4 + 4 * next_case;
    result[at] = (unsigned int)rc;
    result[at + 1] = id_word != 0xFFFFFFFFu;
    result[at + 2] = rc == 0 ? (unsigned int)syscall1_s32(destroy_sc, id_word) : NOT_RUN;
    result[at + 3] = 0;
    next_case++;
}

static unsigned int key_hi(void) { return (unsigned int)(KEY_BASE >> 32); }

/* The key and flags words of a case: its own key and flags 1 when it is
 * process-shared, else 0 and 0. */
static void key_words(unsigned int pshared, unsigned int *hi, unsigned int *lo,
                      unsigned int *flags)
{
    int shared = pshared == PROCESS_SHARED;
    *hi = shared ? key_hi() : 0;
    *lo = shared ? next_case : 0;
    *flags = shared ? 1 : 0;
}

static void sem_at(volatile unsigned int *blk, unsigned int words,
                   unsigned int protocol, unsigned int pshared)
{
    unsigned int w[8], hi, lo, flags;
    key_words(pshared, &hi, &lo, &flags);
    w[0] = protocol; w[1] = pshared; w[2] = hi; w[3] = lo;
    w[4] = flags; w[5] = 0; w[6] = 0; w[7] = 0;
    put_words(blk, w, words);
    id_word = 0xFFFFFFFFu;
    record(syscall4_s32(SYS_SEMAPHORE_CREATE, EA(&id_word), EA(blk), 0, 1),
           SYS_SEMAPHORE_DESTROY);
}

static void mutex_at(volatile unsigned int *blk, unsigned int words,
                     unsigned int protocol, unsigned int recursive,
                     unsigned int pshared, unsigned int adaptive)
{
    unsigned int w[10], hi, lo, flags;
    key_words(pshared, &hi, &lo, &flags);
    w[0] = protocol; w[1] = recursive; w[2] = pshared; w[3] = adaptive;
    w[4] = hi; w[5] = lo; w[6] = flags; w[7] = 0; w[8] = 0; w[9] = 0;
    put_words(blk, w, words);
    id_word = 0xFFFFFFFFu;
    record(syscall2_s32(SYS_MUTEX_CREATE, EA(&id_word), EA(blk)), SYS_MUTEX_DESTROY);
}

static void flag_at(volatile unsigned int *blk, unsigned int words,
                    unsigned int protocol, unsigned int pshared,
                    unsigned int flags_word, unsigned int type)
{
    unsigned int w[8], hi, lo, flags;
    key_words(pshared, &hi, &lo, &flags);
    w[0] = protocol; w[1] = pshared; w[2] = hi; w[3] = lo;
    w[4] = flags | flags_word; w[5] = type; w[6] = 0; w[7] = 0;
    put_words(blk, w, words);
    id_word = 0xFFFFFFFFu;
    record(syscall3_s32(SYS_EVENT_FLAG_CREATE, EA(&id_word), EA(blk), 0),
           SYS_EVENT_FLAG_DESTROY);
}

static void cond_at(volatile unsigned int *blk, unsigned int words, unsigned int pshared)
{
    unsigned int w[6], hi, lo, flags;
    key_words(pshared, &hi, &lo, &flags);
    w[0] = pshared; w[1] = flags; w[2] = hi; w[3] = lo; w[4] = 0; w[5] = 0;
    put_words(blk, w, words);
    id_word = 0xFFFFFFFFu;
    record(syscall3_s32(SYS_COND_CREATE, EA(&id_word), helper, EA(blk)), SYS_COND_DESTROY);
}

static void port(unsigned int type, unsigned long long name)
{
    id_word = 0xFFFFFFFFu;
    record(syscall3_s32(SYS_EVENT_PORT_CREATE, EA(&id_word), type, name),
           SYS_EVENT_PORT_DESTROY);
}

#define AT(addr) ((volatile unsigned int *)(unsigned long)(addr))

int main(void)
{
    s32 rc;
    unsigned int i, end;
    static const unsigned int helper_block[10] = {
        FIFO, NOT_RECURSIVE, NOT_PROCESS_SHARED, NOT_ADAPTIVE, 0, 0, 0, 0, 0, 0
    };

    memset((void *)result, 0, sizeof(result));

    rc = syscall3_s32(SYS_MEMORY_ALLOCATE, ALLOC_SIZE, PAGE_SIZE_1M, EA(&alloc_addr));
    if (rc != 0)
        return fail(1, rc);
    end = alloc_addr + ALLOC_SIZE;
    result[2] = (unsigned int)syscall2_s32(SYS_MEMORY_GET_PAGE_ATTRIBUTE, end, EA(page_attr));
    put_words(block, helper_block, 10);
    rc = syscall2_s32(SYS_MUTEX_CREATE, EA(&id_word), EA(block));
    if (rc != 0)
        return fail(2, rc);
    helper = id_word;

    for (i = 0; i < 9; i++)
        sem_at(block, 8, i, NOT_PROCESS_SHARED);
    for (i = 0; i < 6; i++)
        sem_at(block, 8, FIFO, PSHARED[i]);
    for (i = 0; i < 9; i++)
        mutex_at(block, 10, i, NOT_RECURSIVE, NOT_PROCESS_SHARED, NOT_ADAPTIVE);
    for (i = 0; i < 6; i++)
        mutex_at(block, 10, FIFO, RECURSIVE[i], NOT_PROCESS_SHARED, NOT_ADAPTIVE);
    for (i = 0; i < 6; i++)
        mutex_at(block, 10, FIFO, NOT_RECURSIVE, PSHARED[i], NOT_ADAPTIVE);
    for (i = 0; i < 9; i++)
        flag_at(block, 8, i, NOT_PROCESS_SHARED, 0, WAITER_SINGLE);
    for (i = 0; i < 6; i++)
        flag_at(block, 8, FIFO, NOT_PROCESS_SHARED, 0, TYPE[i]);
    for (i = 0; i < 6; i++)
        flag_at(block, 8, FIFO, PSHARED[i], 0, WAITER_SINGLE);
    for (i = 0; i < 6; i++)
        cond_at(block, 6, PSHARED[i]);
    for (i = 0; i < 9; i++)
        port(i, 0);
    port(3, KEY_BASE + next_case);

    mutex_at(block, 10, NOT_RECURSIVE, FIFO, NOT_PROCESS_SHARED, NOT_ADAPTIVE);
    mutex_at(block, 10, FIFO, NOT_RECURSIVE, NOT_ADAPTIVE, NOT_PROCESS_SHARED);
    flag_at(block, 8, FIFO, NOT_PROCESS_SHARED, WAITER_SINGLE, 0);

    /* Each block ends at the allocation's last byte. */
    sem_at(AT(end - 32), 8, FIFO, NOT_PROCESS_SHARED);
    mutex_at(AT(end - 40), 10, FIFO, NOT_RECURSIVE, NOT_PROCESS_SHARED, NOT_ADAPTIVE);
    flag_at(AT(end - 32), 8, FIFO, NOT_PROCESS_SHARED, 0, WAITER_SINGLE);
    cond_at(AT(end - 24), 6, NOT_PROCESS_SHARED);
    /* Each block's last 4 bytes lie past it; only the mapped words are
     * written. */
    sem_at(AT(end - 28), 7, FIFO, NOT_PROCESS_SHARED);
    mutex_at(AT(end - 36), 9, FIFO, NOT_RECURSIVE, NOT_PROCESS_SHARED, NOT_ADAPTIVE);
    flag_at(AT(end - 28), 7, FIFO, NOT_PROCESS_SHARED, 0, WAITER_SINGLE);
    cond_at(AT(end - 20), 5, NOT_PROCESS_SHARED);

    (void)syscall1_s32(SYS_MUTEX_DESTROY, helper);
    (void)syscall1_s32(SYS_MEMORY_FREE, alloc_addr);

    write_result();
    return 0;
}
