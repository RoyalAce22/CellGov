/* PPU probe: what sys_mutex_create (100) answers for a null attribute
 * block and for each attribute word out of range, alone and together.
 *
 * Each case starts from one valid attribute block -- FIFO, not
 * recursive, not process-shared, not adaptive, key 0, flags 0 -- and
 * changes the fields the case names. The id word is filled with
 * 0xFFFFFFFF before the call, so the payload shows whether the call
 * stored an id. A case whose create succeeds destroys its mutex with
 * sys_mutex_destroy (101) at once.
 *
 * Cases, in payload order:
 *   0  null attribute pointer
 *   1  the valid block (control)
 *   2  adaptive 0
 *   3  adaptive 0x1000 (SYS_SYNC_ADAPTIVE)
 *   4  adaptive 0x3000
 *   5  adaptive 0xFFFF
 *   6  adaptive 0 and protocol 0
 *   7  process-shared, key 0, flags 1
 *   8  process-shared, key 0x1234, flags 0
 *   9  process-shared, key 0x1234, flags 4
 *   10 process-shared, key 0x1234, flags 1 (control)
 *   11 process-shared, key 0, flags 1, and adaptive 0
 *   12 recursive 0 and adaptive 0
 *   13 not process-shared, key 0x1234, flags 4
 *
 * Payload layout (16 bytes per case, big-endian):
 *   +0  u32 create return code
 *   +4  u32 1 when the call changed the id word, else 0
 *   +8  u32 destroy return code, or 0xFFFFFFFF when the create failed
 *   +12 u32 zero
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_MUTEX_CREATE   100
#define SYS_MUTEX_DESTROY  101

#define FIFO               0x1
#define NOT_RECURSIVE      0x20
#define PROCESS_SHARED     0x100
#define NOT_PROCESS_SHARED 0x200
#define NOT_ADAPTIVE       0x2000

#define ID_FILL 0xFFFFFFFFu
#define NOT_RUN 0xFFFFFFFFu

/* sys_mutex_attribute_t: protocol@0, recursive@4, pshared@8,
 * adaptive@12, ipc_key@16 (u64), flags@24, pad@28, name@32 (8). */
struct MutexAttr {
    unsigned int protocol;
    unsigned int recursive;
    unsigned int pshared;
    unsigned int adaptive;
    unsigned long long ipc_key;
    unsigned int flags;
    unsigned int pad;
    char name[8];
};

struct CaseRecord {
    unsigned int create_rc;
    unsigned int id_written;
    unsigned int destroy_rc;
    unsigned int zero;
};

#define CASES 14

struct ProbeResult {
    struct CaseRecord records[CASES];
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static struct ProbeResult result __attribute__((aligned(128)));
/* volatile: each field is one 32-bit store. Without it the compiler
 * merges two fields into a 64-bit constant built with rldicr, which
 * common/patch_toc.py rewrites into a 32-bit shift that drops the
 * high word. */
static volatile struct MutexAttr attr __attribute__((aligned(8)));
static unsigned int mutex_id __attribute__((aligned(8)));

static void write_result(const struct ProbeResult *r)
{
    unsigned int written;
    unsigned int len = sizeof(*r);
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, r, len, &written);
    CGOV_OUT_FILE_WRITE(r, len);
}

static void base_attr(void)
{
    unsigned int i;
    attr.protocol = FIFO;
    attr.recursive = NOT_RECURSIVE;
    attr.pshared = NOT_PROCESS_SHARED;
    attr.adaptive = NOT_ADAPTIVE;
    attr.ipc_key = 0;
    attr.flags = 0;
    attr.pad = 0;
    for (i = 0; i < sizeof(attr.name); i++)
        attr.name[i] = 0;
}

static void set_case(unsigned int i)
{
    base_attr();
    switch (i) {
    case 2:  attr.adaptive = 0; break;
    case 3:  attr.adaptive = 0x1000; break;
    case 4:  attr.adaptive = 0x3000; break;
    case 5:  attr.adaptive = 0xFFFF; break;
    case 6:  attr.adaptive = 0; attr.protocol = 0; break;
    case 7:  attr.pshared = PROCESS_SHARED; attr.ipc_key = 0; attr.flags = 1; break;
    case 8:  attr.pshared = PROCESS_SHARED; attr.ipc_key = 0x1234; attr.flags = 0; break;
    case 9:  attr.pshared = PROCESS_SHARED; attr.ipc_key = 0x1234; attr.flags = 4; break;
    case 10: attr.pshared = PROCESS_SHARED; attr.ipc_key = 0x1234; attr.flags = 1; break;
    case 11: attr.pshared = PROCESS_SHARED; attr.ipc_key = 0; attr.flags = 1;
             attr.adaptive = 0; break;
    case 12: attr.recursive = 0; attr.adaptive = 0; break;
    case 13: attr.ipc_key = 0x1234; attr.flags = 4; break;
    default: break;
    }
}

static void run_case(unsigned int i, struct CaseRecord *rec)
{
    s32 rc;
    unsigned long attr_ptr = (i == 0) ? 0 : (unsigned long)(const void *)&attr;
    set_case(i);
    mutex_id = ID_FILL;
    rc = syscall2_s32(SYS_MUTEX_CREATE, (unsigned long)&mutex_id, attr_ptr);
    rec->create_rc = (unsigned int)rc;
    rec->id_written = (mutex_id != ID_FILL) ? 1 : 0;
    rec->destroy_rc = NOT_RUN;
    rec->zero = 0;
    if (rc == 0)
        rec->destroy_rc = (unsigned int)syscall1_s32(SYS_MUTEX_DESTROY, mutex_id);
}

int main(void)
{
    unsigned int i;

    memset(&result, 0, sizeof(result));
    for (i = 0; i < CASES; i++)
        run_case(i, &result.records[i]);
    write_result(&result);
    return 0;
}
