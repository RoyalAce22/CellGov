/* SPU program: the atomic commands with an MFC_LSA that is not on a
 * 128-byte boundary.
 *
 * arg1 is the effective address of a 528-byte result block, arg2 the
 * effective address of two consecutive 128-byte lines. The PPU fills the
 * first line with 0x80 + i and the second with zeros.
 *
 * A local-store buffer of two lines starts as i ^ 0x55. Then:
 *   1. getllar the first line to buf + 0x30;
 *   2. put the whole buffer to the result block;
 *   3. add one to every buffer byte, then putllc the first line from
 *      buf + 0x50;
 *   4. putlluc the second line from buf + 0x91.
 * Each byte pattern says which local-store bytes each command used.
 *
 * Result layout (528 bytes):
 *   +0:   u32 status (0), u32 getllar status, u32 putllc status,
 *         u32 putlluc status
 *   +16:  u8[256] the buffer after the getllar
 *   +272: u8[128] the first line after the putllc
 *   +400: u8[128] the second line after the putlluc
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define TAG 1

static volatile unsigned char buf[256] __attribute__((aligned(128)));
static volatile unsigned char check[128] __attribute__((aligned(128)));
static volatile unsigned int header[4] __attribute__((aligned(16)));

static void wait_tag(void)
{
    mfc_write_tag_mask(1 << TAG);
    mfc_read_tag_status_all();
}

int main(unsigned long long arg0,
         unsigned long long arg_result_ea,
         unsigned long long arg_lines_ea,
         unsigned long long arg3)
{
    /* The PPU passes 32-bit addresses; only the low word of each
     * argument carries one. */
    unsigned int result_ea = (unsigned int)arg_result_ea;
    unsigned long long line1 = (unsigned int)arg_lines_ea;
    unsigned long long line2 = line1 + 128;
    int i;
    (void)arg0;
    (void)arg3;

    for (i = 0; i < 256; i++)
        buf[i] = (unsigned char)(i ^ 0x55);

    mfc_getllar((void *)(buf + 0x30), line1, 0, 0);
    header[1] = mfc_read_atomic_status();

    mfc_put((void *)buf, result_ea + 16, 256, TAG, 0, 0);
    wait_tag();

    for (i = 0; i < 256; i++)
        buf[i] = (unsigned char)(buf[i] + 1);
    mfc_putllc((void *)(buf + 0x50), line1, 0, 0);
    header[2] = mfc_read_atomic_status();

    mfc_get((void *)check, line1, 128, TAG, 0, 0);
    wait_tag();
    mfc_put((void *)check, result_ea + 272, 128, TAG, 0, 0);
    wait_tag();

    mfc_putlluc((void *)(buf + 0x91), line2, 0, 0);
    header[3] = mfc_read_atomic_status();

    mfc_get((void *)check, line2, 128, TAG, 0, 0);
    wait_tag();
    mfc_put((void *)check, result_ea + 400, 128, TAG, 0, 0);
    wait_tag();

    header[0] = 0;
    mfc_put((void *)header, result_ea, 16, TAG, 0, 0);
    wait_tag();

    spu_thread_exit(0);
    return 0;
}
