/*
 * Custom CRT0 for PS3 PSL1GHT executables on RPCS3.
 *
 * PSL1GHT's CRT0 uses rldicr (64-bit rotate) to build the TOC
 * address. RPCS3 v0.0.40 mishandles this instruction, causing a
 * branch to a garbage address. This replacement builds the TOC
 * using only 32-bit PPC instructions (lis/ori), then calls
 * PSL1GHT's _initialize which handles _init, main, and exit.
 *
 * Linked with the standard CRT objects (not -nostartfiles) so
 * PSL1GHT's _initialize, __syscalls_init, and libc are available.
 * This file provides a stronger _start that overrides the CRT's.
 */

    .section .rodata
    .globl  _start
    .type   _start, @object
    .align  3
_start:
    .long   __cg_entry      /* code address (32-bit) */
    .long   __cg_entry      /* TOC placeholder       */
    .long   0, 0, 0, 0

    .section .text
    .globl  __cg_entry
    .type   __cg_entry, @function
__cg_entry:
    /* Load TOC base from a data word placed after the code.
     * The build script patches __cg_tocval with .got + 0x8000
     * after linking. */
    bcl     20, 31, 1f
1:
    mflr    %r2
    lwz     %r2, (__cg_tocval - 1b)(%r2)

    /* Save stack end for PSL1GHT */
    lis     %r9, __libc_stack_end@ha
    stw     %r1, __libc_stack_end@l(%r9)

    /* Standard PPC64 function prologue (32-bit stores are fine
     * since the stack is in the low 32-bit address range). */
    mflr    %r0
    stw     %r0, 4(%r1)
    stwu    %r1, -128(%r1)
    li      %r0, 0
    stw     %r0, 0(%r1)

    /* Call main(argc=0, argv=NULL). The microtest uses direct
     * syscalls (sys_tty_write, sys_process_exit) so it does not
     * need the full PSL1GHT constructor chain. */
    li      %r3, 0
    li      %r4, 0
    bl      main
    nop

    /* sys_process_exit(r3) -- syscall 22 */
    li      %r11, 22
    sc
    b       .

    .align  2
    .globl  __cg_tocval
__cg_tocval:
    .long   0xDEADBEEF          /* patched by build script */

    /* Stubs for _init/_fini -- PSL1GHT doesn't provide crti.o.
     * Constructors run via __libc_init_array instead. */
    .section .text
    .globl  _init
    .type   _init, @function
_init:
    blr

    .globl  _fini
    .type   _fini, @function
_fini:
    blr

    .section .bss
    .globl  __libc_stack_end
    .type   __libc_stack_end, @object
    .align  2
__libc_stack_end:
    .space  8

