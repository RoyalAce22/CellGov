/* PPU probe: sys_cond_create (105) with a null attribute pointer, a
 * null id pointer, and a stale mutex id.
 *
 * Payload layout (8 big-endian u32 words, 32 bytes):
 *   0  sc 100 creates the mutex
 *   1  sc 105(&id, mutex, NULL)
 *   2  1 when step 1 wrote the id word, else 0
 *   3  sc 106 destroys that cond when step 1 succeeded, else 0xFFFFFFFF
 *   4  sc 105(NULL, mutex, attr)
 *   5  sc 105(&id, 0xDEADBEEF, attr)
 *   6  sc 105(&id, 0xDEADBEEF, NULL)
 *   7  sc 101 destroys the mutex
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

static const unsigned int mutex_attr[10] __attribute__((aligned(8))) =
    { 1, 0x20, 0x200, 0x2000, 0, 0, 0, 0, 0, 0 };   /* FIFO, not recursive */
static const unsigned int cond_attr[6] __attribute__((aligned(8))) =
    { 0x200, 0, 0, 0, 0, 0 };                        /* private */

static unsigned int result[8] __attribute__((aligned(128)));
static unsigned int id_word __attribute__((aligned(8)));

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

#define EA(p) ((unsigned long)(const void *)(p))

int main(void)
{
    unsigned int mutex_id;

    memset(result, 0, sizeof(result));
    id_word = 0;
    result[0] = (unsigned int)syscall2_s32(SYS_MUTEX_CREATE, EA(&id_word), EA(mutex_attr));
    mutex_id = id_word;

    id_word = 0xFFFFFFFFu;
    result[1] = (unsigned int)syscall3_s32(SYS_COND_CREATE, EA(&id_word), mutex_id, 0);
    result[2] = (id_word != 0xFFFFFFFFu) ? 1 : 0;
    result[3] = 0xFFFFFFFFu;
    if (result[1] == 0)
        result[3] = (unsigned int)syscall1_s32(SYS_COND_DESTROY, id_word);

    result[4] = (unsigned int)syscall3_s32(SYS_COND_CREATE, 0, mutex_id, EA(cond_attr));
    result[5] = (unsigned int)syscall3_s32(SYS_COND_CREATE, EA(&id_word), 0xDEADBEEFu,
                                           EA(cond_attr));
    result[6] = (unsigned int)syscall3_s32(SYS_COND_CREATE, EA(&id_word), 0xDEADBEEFu, 0);
    result[7] = (unsigned int)syscall1_s32(SYS_MUTEX_DESTROY, mutex_id);

    write_result();
    return 0;
}
