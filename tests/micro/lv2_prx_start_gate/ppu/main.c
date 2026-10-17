/* PPU probe: which check _sys_prx_start_module (481) refuses an unknown
 * id with when the option struct is readable. No live module is named.
 *
 * Each case is one raw sc with r3 = id, r4 = 0, r5 = pOpt. The option
 * is five u64 words: size, cmd, entry, res, entry2. Before each case
 * entry and entry2 are seeded with 0x1111... and 0x2222....
 *
 * STALE is liblv2's own id (asked of 496) with bit 23 flipped: the same
 * class and slot, another generation, so it names no loaded module.
 *
 * Cases, in payload order:
 *   0  481(0xDEADBEEF, size 0x28, cmd 1)  the gate probe's case again
 *   1  481(0xDEADBEEF, size 0x20, cmd 1)
 *   2  481(0xDEADBEEF, size 0x28, cmd 2, res 0)
 *   3  481(0xDEADBEEF, size 0x28, cmd 0)
 *   4  481(0xDEADBEEF, size 0,    cmd 1)
 *   5  481(STALE,      size 0x28, cmd 1)
 *   6  482(STALE,      size 0x28, cmd 1)
 *   7  481(STALE,      size 0x28, cmd 2, res 0)
 *   8  481(STALE | 1,  size 0x28, cmd 1)   a low byte set
 *   9  481(0x24800000, size 0x28, cmd 1)   another class byte
 *  10  482(0x24800000, size 0x28, cmd 1)
 *  11  481(0xA3800000, size 0x28, cmd 1)   STALE with bit 31 set
 *  12  482(0xA3800000, size 0x28, cmd 1)
 *  13  481(0x5EADBEEF, size 0x28, cmd 1)   0xDEADBEEF with bit 31 clear
 *
 * Payload layout (232 bytes, big-endian):
 *   0x00  u64 r3 of 496("liblv2")
 *   0x08  per case, u64 r3 then the u64 entry word after the call:
 *         case k at 0x08 + 16k
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PRX_START_MODULE 481
#define SYS_PRX_STOP_MODULE 482
#define SYS_PRX_GET_MODULE_ID_BY_NAME 496
#define BAD_ID 0xDEADBEEFULL
#define CASES 14
#define PAYLOAD (8 + CASES * 16)

static const char LIBLV2[] = "liblv2";
static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned char result[PAYLOAD] __attribute__((aligned(128)));
static volatile u64 opt[5] __attribute__((aligned(16)));

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

static void one(unsigned int k, u64 num, u64 id, u64 size, u64 cmd)
{
    u64 rc;

    opt[0] = size;
    opt[1] = cmd;
    opt[2] = 0x1111111111111111ULL;
    opt[3] = 0;
    opt[4] = 0x2222222222222222ULL;
    rc = (u64)cgov_sc(num, id, 0, (u64)(unsigned long)opt, 0, 0, 0, 0, 0);
    put_u64(8 + 16 * k, rc);
    put_u64(8 + 16 * k + 8, opt[2]);
}

int main(void)
{
    u64 liblv2 = (u64)cgov_sc(SYS_PRX_GET_MODULE_ID_BY_NAME,
                              (u64)(unsigned long)LIBLV2, 0, 0, 0, 0, 0, 0, 0);
    u64 stale = (liblv2 & 0xFFFFFFFFULL) ^ 0x00800000ULL;

    put_u64(0, liblv2);
    one(0, SYS_PRX_START_MODULE, BAD_ID, 0x28, 1);
    one(1, SYS_PRX_START_MODULE, BAD_ID, 0x20, 1);
    one(2, SYS_PRX_START_MODULE, BAD_ID, 0x28, 2);
    one(3, SYS_PRX_START_MODULE, BAD_ID, 0x28, 0);
    one(4, SYS_PRX_START_MODULE, BAD_ID, 0, 1);
    one(5, SYS_PRX_START_MODULE, stale, 0x28, 1);
    one(6, SYS_PRX_STOP_MODULE, stale, 0x28, 1);
    one(7, SYS_PRX_START_MODULE, stale, 0x28, 2);
    one(8, SYS_PRX_START_MODULE, stale | 1, 0x28, 1);
    one(9, SYS_PRX_START_MODULE, 0x24800000ULL, 0x28, 1);
    one(10, SYS_PRX_STOP_MODULE, 0x24800000ULL, 0x28, 1);
    one(11, SYS_PRX_START_MODULE, stale | 0x80000000ULL, 0x28, 1);
    one(12, SYS_PRX_STOP_MODULE, stale | 0x80000000ULL, 0x28, 1);
    one(13, SYS_PRX_START_MODULE, 0x5EADBEEFULL, 0x28, 1);

    write_frame(result, sizeof(result));
    return 0;
}
