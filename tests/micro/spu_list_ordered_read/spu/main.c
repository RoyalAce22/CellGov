/* SPU program for spu_list_ordered_read: when does a list command read
 * its list elements, if a get queued just before it in its tag group is
 * still bringing them into local store?
 *
 * Argument: arg1 (argp) = EA of a 256-byte block the PPU filled:
 *   +0x00  element {size 16, eal = block + 0x80}  (the barrier case)
 *   +0x10  element {size 16, eal = block + 0x90}  (the plain case)
 *   +0x80  16 bytes of 0xF1    +0x90  16 bytes of 0xE2
 *   +0xC0  16 bytes of 0x5A    +0xD0  16 bytes of 0x4B
 *
 * Each case's list starts in local store naming the 0x5A or 0x4B bytes.
 * The SPU gets the block's element over it, then at once issues the list
 * command in the same tag group, and waits for the group:
 *   1. getlb, tag 1, into `barrier`;
 *   2. getl, tag 2, into `plain`.
 * It then puts `barrier` to block + 0x40 and `plain` to block + 0x50.
 *
 * 0xF1 (0xE2) means the list read the element the get brought in; 0x5A
 * (0x4B) means it read the element local store held at its issue.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define TAG_BARRIER 1
#define TAG_PLAIN   2

static mfc_list_element_t barrier_list[2] __attribute__((aligned(16)));
static mfc_list_element_t plain_list[2] __attribute__((aligned(16)));
static unsigned int barrier[4] __attribute__((aligned(16)));
static unsigned int plain[4] __attribute__((aligned(16)));

static void wait_tag(unsigned int tag)
{
    mfc_write_tag_mask(1u << tag);
    mfc_read_tag_status_all();
}

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    unsigned int block = (unsigned int)argp;

    (void)spe_id;
    (void)envp;

    barrier_list[0].notify = 0;
    barrier_list[0].reserved = 0;
    barrier_list[0].size = 16;
    barrier_list[0].eal = block + 0xC0;
    plain_list[0].notify = 0;
    plain_list[0].reserved = 0;
    plain_list[0].size = 16;
    plain_list[0].eal = block + 0xD0;

    mfc_get(barrier_list, argp, 16, TAG_BARRIER, 0, 0);
    mfc_getlb(barrier, argp, barrier_list, 8, TAG_BARRIER, 0, 0);
    wait_tag(TAG_BARRIER);

    mfc_get(plain_list, argp + 0x10, 16, TAG_PLAIN, 0, 0);
    mfc_getl(plain, argp, plain_list, 8, TAG_PLAIN, 0, 0);
    wait_tag(TAG_PLAIN);

    mfc_put(barrier, argp + 0x40, 16, TAG_BARRIER, 0, 0);
    mfc_put(plain, argp + 0x50, 16, TAG_BARRIER, 0, 0);
    wait_tag(TAG_BARRIER);

    spu_thread_exit(0);
    return 0;
}
