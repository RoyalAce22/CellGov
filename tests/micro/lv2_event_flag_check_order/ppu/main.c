/* PPU probe: which check sys_event_flag_wait (85) and
 * sys_event_flag_trywait (86) make first, the id or the mode.
 *
 * Each case is one raw `sc` with r3 = id, r4 = bits (1), r5 = mode,
 * r6 = &result and, for 85, r7 = timeout (0). The result word is
 * filled with 0xDEADBEEFCAFEF00D before the call, so the payload shows
 * whether the call stored through it. No case can park: every case has
 * a bad id, a bad mode, or both.
 *
 * Steps:
 *   1. sys_event_flag_create (82) one flag, init bits 0.
 *   2. Run the twelve cases below, six per syscall.
 *   3. sys_event_flag_destroy (83) the flag.
 *
 * Cases, in payload order, for 85 and then for 86:
 *   0  id 0           mode 0      (both bad)
 *   1  id 0xDEADBEEF  mode 0      (both bad)
 *   2  id 0           mode AND    (id bad only)
 *   3  id 0           mode AND|OR (both bad)
 *   4  live id        mode 0      (mode bad only)
 *   5  live id        mode AND|OR (mode bad only)
 *
 * Payload layout (200 bytes, big-endian):
 *   +0   u32 step that failed (0 = none; 1 = create, 3 = destroy)
 *   +4   u32 that step's return code
 *   +8   12 records of 16 bytes, case i of syscall 85 at 8 + 16*i and
 *        case i of syscall 86 at 8 + 16*(6+i):
 *          +0 u32 return code
 *          +4 u32 zero
 *          +8 u64 the result word after the call
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_EVENT_FLAG_CREATE   82
#define SYS_EVENT_FLAG_DESTROY  83
#define SYS_EVENT_FLAG_WAIT     85
#define SYS_EVENT_FLAG_TRYWAIT  86

#define MODE_AND 0x01
#define MODE_OR  0x02

#define RESULT_FILL 0xDEADBEEFCAFEF00DULL

/* id kinds a case names. */
#define ID_ZERO  0
#define ID_JUNK  1
#define ID_LIVE  2

#define CASES 6

/* sys_event_flag_attribute_t: protocol@0 u32, pshared@4 u32,
 * ipc_key@8 u64, flags@16 s32, type@20 s32, name@24; FIFO, not
 * process-shared, one waiter. */
static const struct {
    unsigned int protocol;
    unsigned int pshared;
    unsigned long long ipc_key;
    unsigned int flags;
    unsigned int type;
    char name[8];
} flag_attr __attribute__((aligned(8))) = { 1, 0x200, 0, 0, 0x10000, "" };

static const struct {
    unsigned int id_kind;
    unsigned int mode;
} cases[CASES] = {
    { ID_ZERO, 0 },
    { ID_JUNK, 0 },
    { ID_ZERO, MODE_AND },
    { ID_ZERO, MODE_AND | MODE_OR },
    { ID_LIVE, 0 },
    { ID_LIVE, MODE_AND | MODE_OR },
};

struct CaseRecord {
    unsigned int rc;
    unsigned int zero;
    unsigned long long result;
};

struct ProbeResult {
    unsigned int failed_step;
    unsigned int failed_rc;
    struct CaseRecord records[2 * CASES];
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static struct ProbeResult result __attribute__((aligned(128)));
static unsigned int flag_id __attribute__((aligned(8)));
static volatile unsigned long long result_word __attribute__((aligned(8)));

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

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    result.failed_step = step;
    result.failed_rc = (unsigned int)rc;
    write_result(&result);
    return (int)step;
}

static unsigned int case_id(unsigned int kind)
{
    switch (kind) {
    case ID_ZERO: return 0;
    case ID_JUNK: return 0xDEADBEEFu;
    default:      return flag_id;
    }
}

static void run_case(unsigned int num, unsigned int i, struct CaseRecord *rec)
{
    s32 rc;
    result_word = RESULT_FILL;
    if (num == SYS_EVENT_FLAG_WAIT)
        rc = syscall5_s32(num, case_id(cases[i].id_kind), 1, cases[i].mode,
                          (unsigned long)&result_word, 0);
    else
        rc = syscall4_s32(num, case_id(cases[i].id_kind), 1, cases[i].mode,
                          (unsigned long)&result_word);
    rec->rc = (unsigned int)rc;
    rec->zero = 0;
    rec->result = result_word;
}

int main(void)
{
    s32 ret;
    unsigned int i;

    memset(&result, 0, sizeof(result));

    ret = syscall3_s32(SYS_EVENT_FLAG_CREATE,
        (unsigned long)&flag_id, (unsigned long)&flag_attr, 0);
    if (ret != 0)
        return fail(1, ret);

    for (i = 0; i < CASES; i++)
        run_case(SYS_EVENT_FLAG_WAIT, i, &result.records[i]);
    for (i = 0; i < CASES; i++)
        run_case(SYS_EVENT_FLAG_TRYWAIT, i, &result.records[CASES + i]);

    ret = syscall1_s32(SYS_EVENT_FLAG_DESTROY, flag_id);
    if (ret != 0)
        return fail(3, ret);

    write_result(&result);
    return 0;
}
