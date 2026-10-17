/* PPU probe: which process-shared cond creates sys_cond_create (105)
 * accepts, by the mutex it binds and the attach flags it carries.
 *
 * Two helper mutexes: M1 not process-shared, M2 process-shared (key
 * KEY_BASE + 0x100, flags 1). A process-shared cond case sets a key of
 * its own (KEY_BASE + its case number). The id word is filled with
 * 0xFFFFFFFF before each create; a create that succeeds is destroyed at
 * once.
 *
 * Cases, in payload order:
 *   0  process-shared, flags 1, on M1
 *   1  process-shared, flags 1, on M2
 *   2  process-shared, flags 0, on M2
 *   3  process-shared, flags 3, on M2
 *   4  not process-shared, on M2
 *   5  process-shared, key 0, flags 1, on M2
 *
 * Payload layout (big-endian u32 words):
 *   +0   failed step (0 = none; 1 = M1, 2 = M2), its return code, the
 *        M1 destroy, the M2 destroy
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

#define SYS_MUTEX_CREATE   100
#define SYS_MUTEX_DESTROY  101
#define SYS_COND_CREATE    105
#define SYS_COND_DESTROY   106

#define FIFO               0x1
#define NOT_RECURSIVE      0x20
#define PROCESS_SHARED     0x100
#define NOT_PROCESS_SHARED 0x200
#define NOT_ADAPTIVE       0x2000

#define KEY_HI   0x43474F00u
#define NOT_RUN  0xFFFFFFFFu
#define CASES    6

static volatile unsigned int result[4 + 4 * CASES] __attribute__((aligned(128)));
static volatile unsigned int id_word __attribute__((aligned(8)));
static volatile unsigned int block[10] __attribute__((aligned(8)));

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

/* A mutex: FIFO, not recursive, not adaptive, `pshared` with key word
 * `key_lo` and flags 1 when process-shared. */
static s32 mutex(unsigned int pshared, unsigned int key_lo, unsigned int *id)
{
    int shared = pshared == PROCESS_SHARED;
    s32 rc;
    block[0] = FIFO;
    block[1] = NOT_RECURSIVE;
    block[2] = pshared;
    block[3] = NOT_ADAPTIVE;
    block[4] = shared ? KEY_HI : 0;
    block[5] = shared ? key_lo : 0;
    block[6] = shared ? 1 : 0;
    block[7] = 0;
    block[8] = 0;
    block[9] = 0;
    id_word = 0xFFFFFFFFu;
    rc = syscall2_s32(SYS_MUTEX_CREATE, EA(&id_word), EA(block));
    *id = id_word;
    return rc;
}

/* Case k: a cond with `pshared`, key words `hi`/`lo` and `flags` on
 * mutex `m`. */
static void cond(unsigned int k, unsigned int m, unsigned int pshared,
                 unsigned int hi, unsigned int lo, unsigned int flags)
{
    unsigned int at = 4 + 4 * k;
    s32 rc;
    block[0] = pshared;
    block[1] = flags;
    block[2] = hi;
    block[3] = lo;
    block[4] = 0;
    block[5] = 0;
    id_word = 0xFFFFFFFFu;
    rc = syscall3_s32(SYS_COND_CREATE, EA(&id_word), m, EA(block));
    result[at] = (unsigned int)rc;
    result[at + 1] = id_word != 0xFFFFFFFFu;
    result[at + 2] = rc == 0 ? (unsigned int)syscall1_s32(SYS_COND_DESTROY, id_word) : NOT_RUN;
    result[at + 3] = 0;
}

int main(void)
{
    s32 rc;
    unsigned int m1, m2;

    memset((void *)result, 0, sizeof(result));

    rc = mutex(NOT_PROCESS_SHARED, 0, &m1);
    if (rc != 0)
        return fail(1, rc);
    rc = mutex(PROCESS_SHARED, 0x100, &m2);
    if (rc != 0) {
        (void)syscall1_s32(SYS_MUTEX_DESTROY, m1);
        return fail(2, rc);
    }

    cond(0, m1, PROCESS_SHARED, KEY_HI, 0, 1);
    cond(1, m2, PROCESS_SHARED, KEY_HI, 1, 1);
    cond(2, m2, PROCESS_SHARED, KEY_HI, 2, 0);
    cond(3, m2, PROCESS_SHARED, KEY_HI, 3, 3);
    cond(4, m2, NOT_PROCESS_SHARED, 0, 0, 0);
    cond(5, m2, PROCESS_SHARED, 0, 0, 1);

    result[2] = (unsigned int)syscall1_s32(SYS_MUTEX_DESTROY, m1);
    result[3] = (unsigned int)syscall1_s32(SYS_MUTEX_DESTROY, m2);

    write_result();
    return 0;
}
