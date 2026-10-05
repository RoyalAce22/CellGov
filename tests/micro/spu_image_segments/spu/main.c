/* SPU program: report what the kernel loaded at the test segments of a
 * hand-built SPU image.
 *
 * arg1 is the effective address of the result block. The program puts
 * 16 bytes from each of COPY_LS, FILL_LS, FILL2_LS and UNTOUCHED_LS at
 * +96 of it (ppu/main.c lists the layout), then exits.
 */

#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define COPY_LS      0x30000u
#define FILL_LS      0x31000u
#define FILL2_LS     0x31010u
#define UNTOUCHED_LS 0x32000u
#define TAG          1

static volatile unsigned int out[16] __attribute__((aligned(16)));

static void take(unsigned int at, unsigned int ls)
{
    volatile unsigned int *from = (volatile unsigned int *)ls;
    int i;
    for (i = 0; i < 4; i++)
        out[at + i] = from[i];
}

int main(unsigned long long arg0,
         unsigned long long arg_result_ea,
         unsigned long long arg2,
         unsigned long long arg3)
{
    (void)arg0;
    (void)arg2;
    (void)arg3;

    take(0, COPY_LS);
    take(4, FILL_LS);
    take(8, FILL2_LS);
    take(12, UNTOUCHED_LS);
    mfc_put((void *)out, (unsigned int)arg_result_ea + 96, sizeof(out), TAG, 0, 0);
    mfc_write_tag_mask(1 << TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
