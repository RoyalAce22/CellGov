/* PPU probe: the class byte of the ids a cond, an event port and an
 * lwcond take, next to a mutex and an lwmutex, and the id a mutex takes
 * once every object is destroyed.
 *
 * Payload words (big-endian u32):
 *    0  sc 100 mutex create        1  its id
 *    2  sc 105 cond create         3  its id
 *    4  sc 134 event port create   5  its id
 *    6  sc 95 lwmutex create       7  its id
 *    8  sc 111 lwcond create       9  its id
 *   10  sc 112 lwcond destroy
 *   11  sc 96 lwmutex destroy
 *   12  sc 135 event port destroy
 *   13  sc 106 cond destroy
 *   14  sc 101 mutex destroy
 *   15  sc 100 mutex create again 16  its id
 *   17  sc 101 destroy it
 *   18  first step that failed, as its word index + 1 (0 = none)
 *   19  0xC0DEC1A5, written last
 *
 * Every object is destroyed before the probe exits.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_LWMUTEX_CREATE       95
#define SYS_LWMUTEX_DESTROY      96
#define SYS_MUTEX_CREATE         100
#define SYS_MUTEX_DESTROY        101
#define SYS_COND_CREATE          105
#define SYS_COND_DESTROY         106
#define SYS_LWCOND_CREATE        111
#define SYS_LWCOND_DESTROY       112
#define SYS_EVENT_PORT_CREATE    134
#define SYS_EVENT_PORT_DESTROY   135

#define SYS_EVENT_PORT_LOCAL     1

static const unsigned int mutex_attr[10] __attribute__((aligned(8))) =
    { 1, 0x20, 0x200, 0x2000, 0, 0, 0, 0, 0, 0 };   /* FIFO, not recursive */
static const unsigned int cond_attr[6] __attribute__((aligned(8))) =
    { 0x200, 0, 0, 0, 0, 0 };                        /* private */

static unsigned char lw_control[32] __attribute__((aligned(8)));
static unsigned char lwcond_control[8] __attribute__((aligned(8)));

static unsigned int result[20] __attribute__((aligned(128)));
static unsigned int id_word __attribute__((aligned(8)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((unsigned long)(const void *)(p))

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

static void note_failure(unsigned int at)
{
    if (result[18] == 0)
        result[18] = at + 1;
}

/* Record the rc in word `at` and the id word in `at + 1`; the id on
 * success, 0 otherwise. */
static unsigned int created(unsigned int at, s32 rc)
{
    result[at] = (unsigned int)rc;
    result[at + 1] = id_word;
    if (rc != 0) {
        note_failure(at);
        return 0;
    }
    return id_word;
}

/* Destroy `id` with syscall `num`, recording the rc in word `at`; a
 * create that failed leaves 0xFFFFFFFF there. */
static void destroyed(unsigned int at, unsigned int num, unsigned int id)
{
    if (id == 0) {
        result[at] = 0xFFFFFFFFu;
        return;
    }
    result[at] = (unsigned int)syscall1_s32(num, id);
    if (result[at] != 0)
        note_failure(at);
}

int main(void)
{
    unsigned int mutex, cond = 0, port, lwmutex, lwcond = 0, again;

    memset(result, 0, sizeof(result));

    id_word = 0;
    mutex = created(0, syscall2_s32(SYS_MUTEX_CREATE, EA(&id_word), EA(mutex_attr)));
    if (mutex) {
        id_word = 0;
        cond = created(2, syscall3_s32(SYS_COND_CREATE, EA(&id_word), mutex, EA(cond_attr)));
    }
    id_word = 0;
    port = created(4, syscall3_s32(SYS_EVENT_PORT_CREATE, EA(&id_word), SYS_EVENT_PORT_LOCAL,
                                   0x1234));
    id_word = 0;
    lwmutex = created(6, syscall6_s32(SYS_LWMUTEX_CREATE, EA(&id_word), 1, EA(lw_control),
                                      0, 0, 0));
    if (lwmutex) {
        id_word = 0;
        lwcond = created(8, syscall6_s32(SYS_LWCOND_CREATE, EA(&id_word), lwmutex,
                                         EA(lwcond_control), 0, 0, 0));
    }

    destroyed(10, SYS_LWCOND_DESTROY, lwcond);
    destroyed(11, SYS_LWMUTEX_DESTROY, lwmutex);
    destroyed(12, SYS_EVENT_PORT_DESTROY, port);
    destroyed(13, SYS_COND_DESTROY, cond);
    destroyed(14, SYS_MUTEX_DESTROY, mutex);

    id_word = 0;
    again = created(15, syscall2_s32(SYS_MUTEX_CREATE, EA(&id_word), EA(mutex_attr)));
    destroyed(17, SYS_MUTEX_DESTROY, again);

    result[19] = 0xC0DEC1A5u;
    write_result();
    return 0;
}
