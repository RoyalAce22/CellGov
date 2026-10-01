/* SPU program for spu_events: arm one SPU event per phase, meet its
 * documented condition, and record whether and how soon it fires.
 *
 * Arguments: arg1 (argp) = EA of the 192-byte result block; arg2
 * (envp) = EA of a 128-byte-aligned region holding the 16-byte flag the
 * PPU polls in its first line and a 4 KiB scratch area from offset 128,
 * whose first line is the one the LR phase reserves.
 *
 * Each phase writes a 16-byte record:
 *   +0  the event status read (masked), 0 when the event never fired
 *   +4  SPU_RdEventStat count polls before it rose (MAX_POLLS: never)
 *   +8  the count after the read
 *   +12 a phase-specific word
 *
 * Phases:
 *   0  start: +0 the decrementer at main's entry, +4 read again, +8 the
 *      event mask the thread starts with, +12 the pending count
 *   1  MS: a multisource sync request with nothing in flight
 *   2  TG: a 16-byte get, then a tag-status update request (any)
 *   3  QV: sixteen 4 KiB gets; +12 the lowest MFC_Cmd count seen
 *   4  SN: a two-element list with stall-and-notify on the first;
 *      +12 the list stall status read
 *   5  TM: the decrementer written 50000; +12 the decrementer after
 *   6  MB: the PPU writes the inbound mailbox; +12 the word read
 *   7  S1: the PPU writes signal notification 1; +12 the word read
 *   8  LR: getllar on the scratch line, then the PPU stores to it;
 *      +12 the getllar atomic status
 *   9  masked latch: MS raised with the mask clear; +0 the count with
 *      the mask clear, +8 the count once MS is enabled, +12 the status
 *   10 stop: after phase 9 acknowledged every event with the mask
 *      clear, +0 and +4 the decrementer before and after a short spin;
 *      then the decrementer written 0x7FFFFFFF, +8 and +12 the same
 *   11 rate: +0 the decrementer when the SPU raises flag 11, +4 when
 *      it raises flag 12 after SPIN nops, +8 SPIN; the PPU reads the
 *      time base at each flag
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define MAX_POLLS  1000000u
#define PHASES     12
#define SPIN       (1u << 28)
#define SHORT_SPIN (1u << 20)

#define TAG_TG    1
#define TAG_QV    2
#define TAG_SN    3
#define TAG_FLAG  4
#define TAG_OUT   5

static unsigned int records[PHASES][4] __attribute__((aligned(128)));
static unsigned int flag[4] __attribute__((aligned(16)));
static unsigned char line[128] __attribute__((aligned(128)));
static unsigned char small[2][16] __attribute__((aligned(16)));
static unsigned char bulk[16][4096] __attribute__((aligned(128)));
static mfc_list_element_t list[2] __attribute__((aligned(8)));

static void quiet(void)
{
    spu_write_event_mask(0);
    spu_write_event_ack(0xFFFFFFFFu);
}

/* Wait for the enabled event count to rise; record the status read. */
static void wait_event(unsigned int *record)
{
    unsigned int polls = 0;
    while (spu_stat_event_status() == 0 && polls < MAX_POLLS)
        polls++;
    record[1] = polls;
    if (polls < MAX_POLLS) {
        record[0] = spu_read_event_status();
        spu_write_event_ack(record[0]);
    }
    record[2] = spu_stat_event_status();
}

static void wait_tags(unsigned int mask)
{
    mfc_write_tag_mask(mask);
    mfc_read_tag_status_all();
}

/* Tell the PPU which phase it should act on. */
static void raise_flag(unsigned long long flag_ea, unsigned int phase)
{
    flag[0] = phase;
    mfc_put(flag, flag_ea, sizeof(flag), TAG_FLAG, 0, 0);
    wait_tags(1u << TAG_FLAG);
}

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    unsigned long long flag_ea;
    unsigned long long scratch_ea;
    unsigned int i;

    /* The start values first, before anything else runs. */
    records[0][0] = spu_read_decrementer();
    records[0][1] = spu_read_decrementer();
    records[0][2] = spu_readch(SPU_RdEventMask);
    records[0][3] = spu_stat_event_status();

    (void)spe_id;
    /* The PPU's region: the flag in its first line, the scratch area
     * from the second, so the flag never shares the reserved line. */
    flag_ea = envp;
    scratch_ea = envp + 128;

    /* 1: MS */
    quiet();
    spu_write_event_mask(MFC_MULTI_SRC_SYNC_EVENT);
    mfc_write_multi_src_sync_request();
    wait_event(records[1]);

    /* 2: TG */
    quiet();
    spu_write_event_mask(MFC_TAG_STATUS_UPDATE_EVENT);
    mfc_get(small[0], scratch_ea, 16, TAG_TG, 0, 0);
    mfc_write_tag_mask(1u << TAG_TG);
    mfc_write_tag_update_any();
    wait_event(records[2]);
    records[2][3] = mfc_read_tag_status();

    /* 3: QV */
    quiet();
    spu_write_event_mask(MFC_COMMAND_QUEUE_AVAILABLE_EVENT);
    records[3][3] = 0xFFFFFFFFu;
    for (i = 0; i < 16; i++) {
        unsigned int free_slots;
        mfc_get(bulk[i], scratch_ea, 4096, TAG_QV, 0, 0);
        free_slots = spu_readchcnt(MFC_Cmd);
        if (free_slots < records[3][3])
            records[3][3] = free_slots;
    }
    wait_event(records[3]);
    wait_tags(1u << TAG_QV);

    /* 4: SN */
    quiet();
    spu_write_event_mask(MFC_LIST_STALL_NOTIFY_EVENT);
    list[0].notify = 1;
    list[0].size = 16;
    list[0].eal = (unsigned int)scratch_ea;
    list[1].notify = 0;
    list[1].size = 16;
    list[1].eal = (unsigned int)scratch_ea + 16;
    mfc_getl(small, scratch_ea, list, sizeof(list), TAG_SN, 0, 0);
    wait_event(records[4]);
    records[4][3] = mfc_read_list_stall_status();
    mfc_write_list_stall_ack(TAG_SN);
    wait_tags(1u << TAG_SN);

    /* 5: TM */
    quiet();
    spu_write_event_mask(MFC_DECREMENTER_EVENT);
    spu_write_decrementer(50000);
    wait_event(records[5]);
    records[5][3] = spu_read_decrementer();

    /* 6: MB, the PPU writes the inbound mailbox */
    quiet();
    spu_write_event_mask(MFC_IN_MBOX_AVAILABLE_EVENT);
    raise_flag(flag_ea, 6);
    wait_event(records[6]);
    if (spu_stat_in_mbox() != 0)
        records[6][3] = spu_read_in_mbox();

    /* 7: S1, the PPU writes signal notification 1 */
    quiet();
    spu_write_event_mask(MFC_SIGNAL_NOTIFY_1_EVENT);
    raise_flag(flag_ea, 7);
    wait_event(records[7]);
    if (spu_stat_signal1() != 0)
        records[7][3] = spu_read_signal1();

    /* 8: LR, the PPU stores to the reserved line */
    quiet();
    spu_write_event_mask(MFC_LLR_LOST_EVENT);
    mfc_getllar(line, scratch_ea, 0, 0);
    records[8][3] = mfc_read_atomic_status();
    raise_flag(flag_ea, 8);
    wait_event(records[8]);

    /* 9: a pending event latches while masked */
    quiet();
    mfc_write_multi_src_sync_request();
    for (i = 0; i < 1000; i++)
        spu_stat_event_status();
    records[9][0] = spu_stat_event_status();
    spu_write_event_mask(MFC_MULTI_SRC_SYNC_EVENT);
    records[9][2] = spu_stat_event_status();
    if (records[9][2] != 0)
        records[9][3] = spu_read_event_status();
    quiet();

    /* 10: does acknowledging Tm with the mask clear stop the
     * decrementer, and does a write start it again? */
    records[10][0] = spu_read_decrementer();
    for (i = 0; i < SHORT_SPIN; i++)
        __asm__ volatile ("nop");
    records[10][1] = spu_read_decrementer();
    spu_write_decrementer(0x7FFFFFFFu);
    records[10][2] = spu_read_decrementer();
    for (i = 0; i < SHORT_SPIN; i++)
        __asm__ volatile ("nop");
    records[10][3] = spu_read_decrementer();

    /* 11: the rate, bracketed by two flags the PPU timestamps */
    records[11][0] = spu_read_decrementer();
    raise_flag(flag_ea, 11);
    for (i = 0; i < SPIN; i++)
        __asm__ volatile ("nop");
    records[11][1] = spu_read_decrementer();
    raise_flag(flag_ea, 12);
    records[11][2] = SPIN;

    mfc_put(records, argp, sizeof(records), TAG_OUT, 0, 0);
    wait_tags(1u << TAG_OUT);

    spu_thread_exit(0);
    return 0;
}
