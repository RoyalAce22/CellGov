/* Child SELF for the process_spawn_wait microtest: write a magic
 * word into its OWN address space (the parent must never see it at
 * the same numeric address), burn a visible number of instructions
 * so parent polls interleave with child execution, then exit 42
 * (CRT0 routes the return value into sys_process_exit). */

#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000)

#define MAGIC_ADDR 0x100
#define MAGIC      0x600DF00Du

/* TOC-referenced global so the linker emits .got (the common
 * patch_toc.py step requires it). */
static volatile unsigned int spin_target = 1000;

int main(void)
{
    volatile unsigned int *magic = (volatile unsigned int *)MAGIC_ADDR;
    volatile unsigned int spin;
    *magic = MAGIC;
    for (spin = 0; spin < spin_target; spin++) { }
    return 42;
}
