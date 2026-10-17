/* SPU program for spu_event_edges: whether the Qv and Sn events follow
 * the edge of their channel counts.
 *
 * Arguments: arg1 (argp) = EA of the 64-byte result block; arg2 (envp)
 * = EA of a 128-byte-aligned 4 KiB scratch region.
 *
 * Each phase writes a 16-byte record:
 *   0  reset, before anything else runs: +0 the SPU_RdEventStat count
 *      with only Qv enabled, +4 the count with every event but Tm, Le,
 *      Me and A enabled, +8 the status read when +4 is not 0 (else 0),
 *      +12 the event mask the thread starts with
 *   1  a second stall with MFC_RdListStallStat unread: list A (tag 3)
 *      stalls and Sn is acknowledged without reading the stall status,
 *      then list B (tag 6) stalls. +0 the first status read, +4 the
 *      second (0 when Sn never came again), +8 the second wait's polls
 *      (MAX_POLLS: never), +12 the stall status read after
 *   2  the same with the stall status read between the two lists:
 *      +0, +4, +8 as phase 1, +12 the stall status read after list B
 *   3  one 16-byte get with Qv enabled, the queue never full: +0 the
 *      count once the get completed, +4 the status read when +0 is not
 *      0 (else 0), +8 the MFC_Cmd count after the get, +12 0
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define MAX_POLLS 1000000u
#define PHASES    4

#define TAG_A     3
#define TAG_B     6
#define TAG_GET   1
#define TAG_OUT   5

#define ALL_BUT_TM_LE_ME_A \
    (MFC_MULTI_SRC_SYNC_EVENT | MFC_LLR_LOST_EVENT | \
     MFC_SIGNAL_NOTIFY_1_EVENT | MFC_SIGNAL_NOTIFY_2_EVENT | \
     MFC_IN_MBOX_AVAILABLE_EVENT | MFC_COMMAND_QUEUE_AVAILABLE_EVENT | \
     MFC_LIST_STALL_NOTIFY_EVENT | MFC_TAG_STATUS_UPDATE_EVENT)

static unsigned int records[PHASES][4] __attribute__((aligned(128)));
static unsigned char data_a[2][16] __attribute__((aligned(16)));
static unsigned char data_b[2][16] __attribute__((aligned(16)));
static unsigned char small[16] __attribute__((aligned(16)));
static mfc_list_element_t list_a[2] __attribute__((aligned(8)));
static mfc_list_element_t list_b[2] __attribute__((aligned(8)));

static void quiet(void)
{
    spu_write_event_mask(0);
    spu_write_event_ack(0xFFFFFFFFu);
}

/* Poll the enabled event count; return the status read (acknowledged)
 * or 0 when it never rose, and the polls in *polls. */
static unsigned int poll_event(unsigned int *polls)
{
    unsigned int n = 0;
    unsigned int status = 0;
    while (spu_stat_event_status() == 0 && n < MAX_POLLS)
        n++;
    if (n < MAX_POLLS) {
        status = spu_read_event_status();
        spu_write_event_ack(status);
    }
    if (polls)
        *polls = n;
    return status;
}

static void wait_tags(unsigned int mask)
{
    mfc_write_tag_mask(mask);
    mfc_read_tag_status_all();
}

/* A two-element list get whose first element carries stall-and-notify. */
static void stalling_list(mfc_list_element_t *list, void *ls,
                          unsigned long long scratch_ea, unsigned int tag)
{
    list[0].notify = 1;
    list[0].size = 16;
    list[0].eal = (unsigned int)scratch_ea;
    list[1].notify = 0;
    list[1].size = 16;
    list[1].eal = (unsigned int)scratch_ea + 16;
    mfc_getl(ls, scratch_ea, list, 2 * sizeof(mfc_list_element_t), tag, 0, 0);
}

static void resume(unsigned int stalled)
{
    if (stalled & (1u << TAG_A))
        mfc_write_list_stall_ack(TAG_A);
    if (stalled & (1u << TAG_B))
        mfc_write_list_stall_ack(TAG_B);
    wait_tags((1u << TAG_A) | (1u << TAG_B));
}

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    unsigned int polls;
    unsigned int stalled;

    (void)spe_id;

    /* 0: reset */
    records[0][3] = spu_readch(SPU_RdEventMask);
    spu_write_event_mask(MFC_COMMAND_QUEUE_AVAILABLE_EVENT);
    records[0][0] = spu_stat_event_status();
    spu_write_event_mask(ALL_BUT_TM_LE_ME_A);
    records[0][1] = spu_stat_event_status();
    records[0][2] = records[0][1] != 0 ? spu_read_event_status() : 0;

    /* 1: list B stalls while list A's stall status is unread */
    quiet();
    spu_write_event_mask(MFC_LIST_STALL_NOTIFY_EVENT);
    stalling_list(list_a, data_a, envp, TAG_A);
    records[1][0] = poll_event(0);
    stalling_list(list_b, data_b, envp, TAG_B);
    records[1][1] = poll_event(&polls);
    records[1][2] = polls;
    stalled = mfc_read_list_stall_status();
    records[1][3] = stalled;
    resume(stalled);

    /* 2: list B stalls after list A's stall status was read */
    quiet();
    spu_write_event_mask(MFC_LIST_STALL_NOTIFY_EVENT);
    stalling_list(list_a, data_a, envp, TAG_A);
    records[2][0] = poll_event(0);
    stalled = mfc_read_list_stall_status();
    stalling_list(list_b, data_b, envp, TAG_B);
    records[2][1] = poll_event(&polls);
    records[2][2] = polls;
    records[2][3] = mfc_read_list_stall_status();
    resume(stalled | records[2][3]);

    /* 3: one get, the queue never full */
    quiet();
    spu_write_event_mask(MFC_COMMAND_QUEUE_AVAILABLE_EVENT);
    mfc_get(small, envp, 16, TAG_GET, 0, 0);
    wait_tags(1u << TAG_GET);
    records[3][0] = spu_stat_event_status();
    records[3][1] = records[3][0] != 0 ? spu_read_event_status() : 0;
    records[3][2] = spu_readchcnt(MFC_Cmd);
    records[3][3] = 0;
    quiet();

    mfc_put(records, argp, sizeof(records), TAG_OUT, 0, 0);
    wait_tags(1u << TAG_OUT);

    spu_thread_exit(0);
    return 0;
}
