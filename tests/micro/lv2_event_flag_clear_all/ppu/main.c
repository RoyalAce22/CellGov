/* PPU probe: the event flag CLEAR_ALL wait mode, the mode with both
 * clear bits, and the event flag object count around a create and a
 * destroy.
 *
 * Steps (every word of the payload is recorded, even after a failure):
 *   1. sc 12(0x98, &n) counts event flag objects.
 *   2. sc 82 creates a flag with initial bits 0xFF; sc 12 again.
 *   3. sc 86 trywait bits 1, mode OR|CLEAR_ALL (0x22); sc 139 reads the
 *      bits back.
 *   4. sc 87 sets 0xFF; sc 86 trywait bits 1, mode OR|CLEAR|CLEAR_ALL
 *      (0x32); sc 139.
 *   5. sc 87 sets 0xFF; sc 86 trywait bits 3, mode AND|CLEAR_ALL
 *      (0x21); sc 139.
 *   6. sc 87 sets 0xFF; sc 85 wait bits 1, mode OR|CLEAR_ALL, timeout
 *      0 (the bits match, so it cannot park); sc 139.
 *   7. sc 83 destroys the flag; sc 12 again.
 *
 * Every u64 the kernel writes is prefilled with all ones, and the
 * payload keeps all 64 bits as two words, high word first.
 *
 * Payload layout (36 big-endian u32 words, 144 bytes):
 *   0  1 when the create failed, else 0
 *   1  the create's return code
 *   2,3   count before the create        4  sc 12 rc
 *   5,6   count after the create         7  sc 12 rc
 *   8     step 3 trywait rc     9,10  its result   11,12 bits after
 *   13    step 4 set rc         14    trywait rc   15,16 result  17,18 bits
 *   19    step 5 set rc         20    trywait rc   21,22 result  23,24 bits
 *   25    step 6 set rc         26    wait rc      27,28 result  29,30 bits
 *   31    destroy rc
 *   32,33 count after the destroy        34  sc 12 rc
 *   35    zero
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PROCESS_GET_NUMBER_OF_OBJECT 12
#define SYS_EVENT_FLAG_CREATE   82
#define SYS_EVENT_FLAG_DESTROY  83
#define SYS_EVENT_FLAG_WAIT     85
#define SYS_EVENT_FLAG_TRYWAIT  86
#define SYS_EVENT_FLAG_SET      87
#define SYS_EVENT_FLAG_GET      139

#define EVENT_FLAG_CLASS 0x98

#define MODE_AND        0x01
#define MODE_OR         0x02
#define MODE_CLEAR      0x10
#define MODE_CLEAR_ALL  0x20

#define FILL 0xFFFFFFFFFFFFFFFFULL

static const unsigned int flag_attr[8] __attribute__((aligned(8))) =
    { 1, 0x200, 0, 0, 0, 0x10000, 0, 0 };   /* FIFO, private, one waiter */

static unsigned int result[36] __attribute__((aligned(128)));
static unsigned int flag_id __attribute__((aligned(8)));
static volatile unsigned long long word64 __attribute__((aligned(8)));

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

static void put64(unsigned int at)
{
    unsigned long long v = word64;
    result[at] = (unsigned int)(v >> 32);
    result[at + 1] = (unsigned int)v;
}

/* sc 12 into result[at], result[at + 1]; its rc into result[rc_at]. */
static void count(unsigned int at, unsigned int rc_at)
{
    word64 = FILL;
    result[rc_at] = (unsigned int)syscall2_s32(SYS_PROCESS_GET_NUMBER_OF_OBJECT,
                                               EVENT_FLAG_CLASS, EA(&word64));
    put64(at);
}

static void read_bits(unsigned int at)
{
    word64 = FILL;
    (void)syscall2_s32(SYS_EVENT_FLAG_GET, flag_id, EA(&word64));
    put64(at);
}

/* One trywait (or a wait with timeout 0) and the bits after it. */
static void one_wait(unsigned int num, unsigned long long bits, unsigned int mode,
                     unsigned int rc_at)
{
    word64 = FILL;
    if (num == SYS_EVENT_FLAG_WAIT)
        result[rc_at] = (unsigned int)syscall5_s32(num, flag_id, bits, mode, EA(&word64), 0);
    else
        result[rc_at] = (unsigned int)syscall4_s32(num, flag_id, bits, mode, EA(&word64));
    put64(rc_at + 1);
    read_bits(rc_at + 3);
}

int main(void)
{
    s32 rc;

    memset(result, 0, sizeof(result));
    count(2, 4);
    rc = syscall3_s32(SYS_EVENT_FLAG_CREATE, EA(&flag_id), EA(flag_attr), 0xFF);
    if (rc != 0) {
        result[0] = 1;
        result[1] = (unsigned int)rc;
        write_result();
        return 1;
    }
    count(5, 7);

    one_wait(SYS_EVENT_FLAG_TRYWAIT, 1, MODE_OR | MODE_CLEAR_ALL, 8);

    result[13] = (unsigned int)syscall2_s32(SYS_EVENT_FLAG_SET, flag_id, 0xFF);
    one_wait(SYS_EVENT_FLAG_TRYWAIT, 1, MODE_OR | MODE_CLEAR | MODE_CLEAR_ALL, 14);

    result[19] = (unsigned int)syscall2_s32(SYS_EVENT_FLAG_SET, flag_id, 0xFF);
    one_wait(SYS_EVENT_FLAG_TRYWAIT, 3, MODE_AND | MODE_CLEAR_ALL, 20);

    result[25] = (unsigned int)syscall2_s32(SYS_EVENT_FLAG_SET, flag_id, 0xFF);
    one_wait(SYS_EVENT_FLAG_WAIT, 1, MODE_OR | MODE_CLEAR_ALL, 26);

    result[31] = (unsigned int)syscall1_s32(SYS_EVENT_FLAG_DESTROY, flag_id);
    count(32, 34);

    write_result();
    return 0;
}
