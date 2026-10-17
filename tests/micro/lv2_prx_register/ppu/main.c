/* PPU probe: what _sys_prx_register_module (484) answers a game process
 * for each declared option size. r3 = 0, r4 = pOpt.
 *
 * The option is 0x40 bytes of zero with only size written (u64 at 0):
 * type 0 asks for no import binding, and the stub pointer and size are
 * zero.
 *
 * Case 0 sends the option in r3 with r4 = 0: a null pOpt.
 * Cases 1-11, in payload order (declared size):
 *   1  0x18
 *   2  0x1C
 *   3  0x20
 *   4  0x24
 *   5  0x28
 *   6  0x2C
 *   7  0x30
 *   8  0x34
 *   9  0x38
 *  10  0x3C
 *  11  0x40
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

static const u64 SIZES[] = {
    0x18, 0x1C, 0x20, 0x24, 0x28, 0x2C, 0x30, 0x34, 0x38, 0x3C, 0x40,
};

#define NCASES (1 + sizeof(SIZES) / sizeof(SIZES[0]))

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

    for (i = 0; i < OPT_BYTES; i++)
        opt[i] = 0;
    *(volatile u64 *)&opt[0] = 0x30;
    result[0] = (u64)cgov_sc(SYS_PRX_REGISTER_MODULE,
                             (u64)(unsigned long)opt, 0, 0, 0, 0, 0, 0, 0);

    for (k = 1; k < NCASES; k++) {
        for (i = 0; i < OPT_BYTES; i++)
            opt[i] = 0;
        *(volatile u64 *)&opt[0] = SIZES[k - 1];
        result[k] = (u64)cgov_sc(SYS_PRX_REGISTER_MODULE, 0,
                                 (u64)(unsigned long)opt, 0, 0, 0, 0, 0, 0);
    }

    write_frame(result, sizeof(result));
    return 0;
}
