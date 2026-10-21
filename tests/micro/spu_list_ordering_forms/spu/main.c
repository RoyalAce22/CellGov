/* SPU program for spu_list_ordering_forms: which ordering forms make a
 * list command, or a command after one, wait for a get queued just before
 * it that brings its list elements into local store?
 *
 * Argument: arg1 (argp) = EA of a 1024-byte block the PPU filled:
 *   +0x000  element {size 16, eal = block + 0x100}  (fence case)
 *   +0x010  element {size 16, eal = block + 0x110}  (queue-barrier case)
 *   +0x020  element {size 16, eal = block + 0x120}  (barrier list)
 *   +0x028  element {size 16, eal = block + 0x130}  (plain list after it)
 *   +0x030  element {size 16, eal = block + 0x140}  (put case)
 *   +0x100.. 16 bytes each of 0xA1, 0xB2, 0xD1, 0xD2, 0xC3  (fresh)
 *   +0x200.. 16 bytes each of 0x1A, 0x2B, 0x3D, 0x4D, 0x5C  (stale)
 *
 * Each list starts in local store naming the stale bytes. The SPU gets the
 * block's elements over it and, with no wait, issues:
 *   1. getlf, tag 3, into fence_buf;
 *   2. a barrier command, then getl in tag 5, into queue_buf;
 *   3. getlb, tag 7, into first_buf, then getl from the second element,
 *      tag 7, into second_buf;
 *   4. getlb, tag 6, into put_buf (which holds 0x6C), then a plain put of
 *      put_buf to block + 0x350 and a putb of it to block + 0x340, tag 6.
 * Each case waits for its tags before the next. The SPU then puts the
 * first four buffers to block + 0x300, 0x310, 0x320 and 0x330.
 *
 * A fresh byte means the command read what the get brought in; a stale
 * byte means it read what local store held at its issue; 0x6C at
 * block + 0x350 means the plain put read put_buf before the getlb landed.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define TAG_OUT     1
#define TAG_FENCE   3
#define TAG_BEFORE  4
#define TAG_AFTER   5
#define TAG_PUT     6
#define TAG_GROUP   7

static mfc_list_element_t fence_list[2] __attribute__((aligned(16)));
static mfc_list_element_t queue_list[2] __attribute__((aligned(16)));
static mfc_list_element_t group_list[2] __attribute__((aligned(16)));
static mfc_list_element_t put_list[2] __attribute__((aligned(16)));
static unsigned int fence_buf[4] __attribute__((aligned(16)));
static unsigned int queue_buf[4] __attribute__((aligned(16)));
static unsigned int first_buf[4] __attribute__((aligned(16)));
static unsigned int second_buf[4] __attribute__((aligned(16)));
static unsigned int put_buf[4] __attribute__((aligned(16)));

static void wait_tags(unsigned int mask)
{
    mfc_write_tag_mask(mask);
    mfc_read_tag_status_all();
}

static void stale(mfc_list_element_t *element, unsigned int eal)
{
    element->notify = 0;
    element->reserved = 0;
    element->size = 16;
    element->eal = eal;
}

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    unsigned int block = (unsigned int)argp;
    unsigned int i;

    (void)spe_id;
    (void)envp;

    stale(&fence_list[0], block + 0x200);
    stale(&queue_list[0], block + 0x210);
    stale(&group_list[0], block + 0x220);
    stale(&group_list[1], block + 0x230);
    stale(&put_list[0], block + 0x240);
    for (i = 0; i < 4; i++)
        put_buf[i] = 0x6C6C6C6C;

    mfc_get(fence_list, argp + 0x00, 16, TAG_FENCE, 0, 0);
    mfc_getlf(fence_buf, argp, fence_list, 8, TAG_FENCE, 0, 0);
    wait_tags(1u << TAG_FENCE);

    mfc_get(queue_list, argp + 0x10, 16, TAG_BEFORE, 0, 0);
    mfc_barrier(TAG_BEFORE);
    mfc_getl(queue_buf, argp, queue_list, 8, TAG_AFTER, 0, 0);
    wait_tags((1u << TAG_BEFORE) | (1u << TAG_AFTER));

    mfc_get(group_list, argp + 0x20, 16, TAG_GROUP, 0, 0);
    mfc_getlb(first_buf, argp, &group_list[0], 8, TAG_GROUP, 0, 0);
    mfc_getl(second_buf, argp, &group_list[1], 8, TAG_GROUP, 0, 0);
    wait_tags(1u << TAG_GROUP);

    mfc_get(put_list, argp + 0x30, 16, TAG_PUT, 0, 0);
    mfc_getlb(put_buf, argp, put_list, 8, TAG_PUT, 0, 0);
    mfc_put(put_buf, argp + 0x350, 16, TAG_PUT, 0, 0);
    mfc_putb(put_buf, argp + 0x340, 16, TAG_PUT, 0, 0);
    wait_tags(1u << TAG_PUT);

    mfc_put(fence_buf, argp + 0x300, 16, TAG_OUT, 0, 0);
    mfc_put(queue_buf, argp + 0x310, 16, TAG_OUT, 0, 0);
    mfc_put(first_buf, argp + 0x320, 16, TAG_OUT, 0, 0);
    mfc_put(second_buf, argp + 0x330, 16, TAG_OUT, 0, 0);
    wait_tags(1u << TAG_OUT);

    spu_thread_exit(0);
    return 0;
}
