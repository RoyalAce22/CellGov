/* PPU probe: which declared sizes _sys_prx_get_module_list (494)
 * accepts. r3 = flags, r4 = pInfo. Nothing is loaded or created.
 *
 * Before each case the 0x28-byte struct is filled with FILL, then size
 * is written at 0 (u64), max = 8 at 0xC and idlist at 0x14. The word at
 * 8, count at 0x10 and every byte from 0x18 keep the fill. The idlist
 * array holds 64 slots of 0xCCCCCCCC, so a kernel that reads max from
 * another offset cannot write past it.
 *
 * Cases, in payload order (flags, size, fill):
 *   0  2, 0x00, 0x00
 *   1  2, 0x18, 0x00
 *   2  2, 0x1C, 0x00
 *   3  2, 0x24, 0x00
 *   4  2, 0x30, 0x00
 *   5  2, 0x40, 0x00
 *   6  0, 0x28, 0x00   no fill asked, a refused size
 *
 * Payload layout (big-endian), 112 bytes per case:
 *   +0x00 u64 r3
 *   +0x08 the 0x28-byte struct after the call
 *   +0x30 idlist slots 0-15, u32 each
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PRX_GET_MODULE_LIST 494

#define LIST_STRUCT 0x28
#define LIST_SLOTS 64
#define LIST_KEPT 16
#define CASE_BYTES (8 + LIST_STRUCT + LIST_KEPT * 4)

struct list_case {
    u64 flags;
    u64 size;
    unsigned char fill;
    int null_info;
};

static const struct list_case CASES[] = {
    { 2, 0x00, 0x00, 0 },
    { 2, 0x18, 0x00, 0 },
    { 2, 0x1C, 0x00, 0 },
    { 2, 0x24, 0x00, 0 },
    { 2, 0x30, 0x00, 0 },
    { 2, 0x40, 0x00, 0 },
    { 0, 0x28, 0x00, 0 },
};

#define NCASES (sizeof(CASES) / sizeof(CASES[0]))

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned char result[NCASES * CASE_BYTES] __attribute__((aligned(128)));
static volatile unsigned char info[LIST_STRUCT] __attribute__((aligned(16)));
static volatile u32 ids[LIST_SLOTS] __attribute__((aligned(16)));

static void write_frame(const volatile void *data, unsigned int len)
{
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)data, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)data, len);
}

static void put_u64(unsigned int off, u64 v)
{
    unsigned int i;
    for (i = 0; i < 8; i++)
        result[off + i] = (unsigned char)(v >> (56 - 8 * i));
}

static void put_u32(unsigned int off, u32 v)
{
    unsigned int i;
    for (i = 0; i < 4; i++)
        result[off + i] = (unsigned char)(v >> (24 - 8 * i));
}

static void run_case(unsigned int k, const struct list_case *c)
{
    unsigned int base = k * CASE_BYTES;
    unsigned int i;
    u64 rc;

    for (i = 0; i < LIST_STRUCT; i++)
        info[i] = c->fill;
    for (i = 0; i < LIST_SLOTS; i++)
        ids[i] = 0xCCCCCCCCU;
    *(volatile u64 *)&info[0] = c->size;
    *(volatile u32 *)&info[0xC] = 8;
    *(volatile u32 *)&info[0x14] = (u32)(unsigned long)ids;

    rc = (u64)cgov_sc(SYS_PRX_GET_MODULE_LIST, c->flags,
                      c->null_info ? 0 : (u64)(unsigned long)info,
                      0, 0, 0, 0, 0, 0);
    put_u64(base, rc);
    for (i = 0; i < LIST_STRUCT; i++)
        result[base + 8 + i] = info[i];
    for (i = 0; i < LIST_KEPT; i++)
        put_u32(base + 8 + LIST_STRUCT + i * 4, ids[i]);
}

int main(void)
{
    unsigned int k;

    for (k = 0; k < NCASES; k++)
        run_case(k, &CASES[k]);

    write_frame(result, sizeof(result));
    return 0;
}
