/* PPU probe: what _sys_prx_register_module (484) answers a game process
 * for each module type. r3 = 0, r4 = pOpt.
 *
 * The option is 0x40 bytes of zero with size 0x30 (u64 at 0) and the
 * type (u64 at 8) written. Every table pointer and size is zero, so a
 * type that asks for an import bind names an empty table.
 *
 * Cases, in payload order (type):
 *   0  0
 *   1  1  (bit 0: bind the caller's own import table)
 *   2  2
 *   3  3  (liblv2.sprx's own bind call sends this type)
 *   4  3  again, after every call above
 *
 * Payload layout: per case, a big-endian u64 r3.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PRX_REGISTER_MODULE 484
#define OPT_BYTES 0x40

static const u64 TYPES[] = { 0, 1, 2, 3, 3 };

#define NCASES (sizeof(TYPES) / sizeof(TYPES[0]))

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile u64 result[NCASES] __attribute__((aligned(128)));
static volatile unsigned char opt[OPT_BYTES] __attribute__((aligned(16)));

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

int main(void)
{
    unsigned int k, i;

    for (k = 0; k < NCASES; k++) {
        for (i = 0; i < OPT_BYTES; i++)
            opt[i] = 0;
        *(volatile u64 *)&opt[0] = 0x30;
        *(volatile u64 *)&opt[8] = TYPES[k];
        result[k] = (u64)cgov_sc(SYS_PRX_REGISTER_MODULE, 0,
                                 (u64)(unsigned long)opt, 0, 0, 0, 0, 0, 0);
    }

    write_frame(result, sizeof(result));
    return 0;
}
