/* SPU program for spu_ea_bound: one MFC access to an effective address
 * the PPU names, on an SPU thread of a group with an exception event.
 *
 * Arguments: arg1 (argp) = the effective address; arg2 (envp) = the
 * access: 0 a 16-byte get, 1 a getllar.
 *
 * An access that completes exits the thread with EXIT_DONE; one the MFC
 * refuses leaves the thread waiting until the PPU terminates the group.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define TAG       1
#define EXIT_DONE 0x600D

static unsigned char line[128] __attribute__((aligned(128)));

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    (void)spe_id;

    if (envp == 0) {
        mfc_get(line, argp, 16, TAG, 0, 0);
        mfc_write_tag_mask(1u << TAG);
        mfc_read_tag_status_all();
    } else {
        mfc_getllar(line, argp, 0, 0);
        mfc_read_atomic_status();
    }

    spu_thread_exit(EXIT_DONE);
    return 0;
}
