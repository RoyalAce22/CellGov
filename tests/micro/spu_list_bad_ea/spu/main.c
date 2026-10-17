/* SPU program for spu_list_bad_ea: a list put of three elements whose
 * second is bad, on an SPU thread of a group with an exception event.
 *
 * Arguments: arg1 (argp) = EA of a 20 KB buffer of zeros.
 *
 * The list puts from LS `src`, tag 1:
 *   element 1: 16 bytes to buffer + 0x100
 *   element 2: BAD_SIZE bytes to buffer + BAD_OFFSET
 *   element 3: 16 bytes to buffer + 0x200
 * Each element's data starts at the next quadword of `src` after the
 * last; every word of `src` holds its own LS address.
 *
 * The SPU then waits for tag 1. A list that ends cleanly completes it
 * and the SPU exits with status 0; one the MFC refuses leaves the SPU
 * waiting until the PPU terminates the group.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#ifndef BAD_SIZE
#define BAD_SIZE   0x4010u
#endif
#ifndef BAD_OFFSET
#define BAD_OFFSET 0x180u
#endif

#define TAG 1

static unsigned int src[(0x4040 + 32) / 4] __attribute__((aligned(128)));
static mfc_list_element_t list[3] __attribute__((aligned(8)));

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    unsigned int i;

    (void)spe_id;
    (void)envp;

    for (i = 0; i < sizeof(src) / 4; i++)
        src[i] = (unsigned int)&src[i];

    list[0].notify = 0;
    list[0].size = 16;
    list[0].eal = (unsigned int)argp + 0x100;
    list[1].notify = 0;
    list[1].size = BAD_SIZE;
    list[1].eal = (unsigned int)argp + BAD_OFFSET;
    list[2].notify = 0;
    list[2].size = 16;
    list[2].eal = (unsigned int)argp + 0x200;

    mfc_putl(src, argp, list, sizeof(list), TAG, 0, 0);
    mfc_write_tag_mask(1u << TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
