/* PPU probe: when sys_mutex_destroy (101) of a mutex refuses with EPERM
 * after the conds bound to it are destroyed. Each case binds one kind of
 * cond (sys_cond_create, 105) to a fresh mutex, destroys the cond
 * (106), then destroys the mutex.
 *
 * A process-shared mutex is FIFO, not recursive, not adaptive, with key
 * KEY_HI:0x200+case and flags 1. A process-shared cond carries key
 * KEY_HI:0x300+case.
 *
 * Cases:
 *   0  shared mutex, no cond
 *   1  shared mutex, process-shared cond, flags 1
 *   2  shared mutex, process-shared cond, flags 3
 *   3  shared mutex, not-shared cond
 *   4  shared mutex, process-shared cond with flags 0 (a refused create)
 *   5  shared mutex, process-shared cond, flags 1, mutex destroyed while
 *      the cond lives, then the cond, then the mutex
 *   6  not-shared mutex, not-shared cond
 *   7  not-shared mutex, process-shared cond, flags 1 (a refused create)
 *
 * Payload layout (big-endian u32 words), case k at 8k:
 *   +0  mutex create rc
 *   +1  cond create rc (NOT_RUN in case 0)
 *   +2  mutex destroy while the cond lives (NOT_RUN except case 5)
 *   +3  cond destroy rc (NOT_RUN when the create failed or none ran)
 *   +4  mutex destroy rc
 *   +5  a second mutex destroy, when +4 was not 0
 *   +6  sys_mutex_trylock (103), when +4 was not 0
 *   +7  sys_mutex_unlock (104), when +6 was 0
 * NOT_RUN is 0xFFFFFFFF.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_MUTEX_CREATE   100
#define SYS_MUTEX_DESTROY  101
#define SYS_MUTEX_TRYLOCK  103
#define SYS_MUTEX_UNLOCK   104
#define SYS_COND_CREATE    105
#define SYS_COND_DESTROY   106

#define FIFO               0x1
#define NOT_RECURSIVE      0x20
#define PROCESS_SHARED     0x100
#define NOT_PROCESS_SHARED 0x200
#define NOT_ADAPTIVE       0x2000

#define KEY_HI   0x43474F00u
#define NOT_RUN  0xFFFFFFFFu
#define CASES    8
#define NONE     0

static volatile unsigned int result[8 * CASES] __attribute__((aligned(128)));
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

#define EA(p) ((unsigned long)(const void *)(p))

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

static s32 cond(unsigned int m, unsigned int pshared, unsigned int key_lo, unsigned int flags,
                unsigned int *id)
{
    int shared = pshared == PROCESS_SHARED;
    s32 rc;
    block[0] = pshared;
    block[1] = flags;
    block[2] = shared ? KEY_HI : 0;
    block[3] = shared ? key_lo : 0;
    block[4] = 0;
    block[5] = 0;
    id_word = 0xFFFFFFFFu;
    rc = syscall3_s32(SYS_COND_CREATE, EA(&id_word), m, EA(block));
    *id = id_word;
    return rc;
}

/* Case k: mutex `mshared`, then a cond `cshared` with `flags` unless
 * `cshared` is NONE; `early` destroys the mutex while the cond lives. */
static void run_case(unsigned int k, unsigned int mshared, unsigned int cshared,
                     unsigned int flags, int early)
{
    unsigned int at = 8 * k;
    unsigned int m, c = 0;
    s32 rc, crc = -1;
    unsigned int i;

    for (i = 0; i < 8; i++)
        result[at + i] = NOT_RUN;
    rc = mutex(mshared, 0x200 + k, &m);
    result[at] = (unsigned int)rc;
    if (rc != 0)
        return;
    if (cshared != NONE) {
        crc = cond(m, cshared, 0x300 + k, flags, &c);
        result[at + 1] = (unsigned int)crc;
    }
    if (early && crc == 0)
        result[at + 2] = (unsigned int)syscall1_s32(SYS_MUTEX_DESTROY, m);
    if (crc == 0)
        result[at + 3] = (unsigned int)syscall1_s32(SYS_COND_DESTROY, c);
    rc = syscall1_s32(SYS_MUTEX_DESTROY, m);
    result[at + 4] = (unsigned int)rc;
    if (rc == 0)
        return;
    result[at + 5] = (unsigned int)syscall1_s32(SYS_MUTEX_DESTROY, m);
    rc = syscall1_s32(SYS_MUTEX_TRYLOCK, m);
    result[at + 6] = (unsigned int)rc;
    if (rc == 0)
        result[at + 7] = (unsigned int)syscall1_s32(SYS_MUTEX_UNLOCK, m);
}

int main(void)
{
    memset((void *)result, 0, sizeof(result));

    run_case(0, PROCESS_SHARED, NONE, 0, 0);
    run_case(1, PROCESS_SHARED, PROCESS_SHARED, 1, 0);
    run_case(2, PROCESS_SHARED, PROCESS_SHARED, 3, 0);
    run_case(3, PROCESS_SHARED, NOT_PROCESS_SHARED, 0, 0);
    run_case(4, PROCESS_SHARED, PROCESS_SHARED, 0, 0);
    run_case(5, PROCESS_SHARED, PROCESS_SHARED, 1, 1);
    run_case(6, NOT_PROCESS_SHARED, NOT_PROCESS_SHARED, 0, 0);
    run_case(7, NOT_PROCESS_SHARED, PROCESS_SHARED, 1, 0);

    write_result();
    return 0;
}
