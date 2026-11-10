@ Start-up for the decomp-dj GBA test ROM: the cartridge header, stacks, copying the IWRAM
@ section out of ROM, clearing .bss, and calling main (Thumb). ARM code.
    .syntax unified
    .section .crt0, "ax"
    .arm
    .global _start
    .type _start, %function
_start:
    b       reset
    .space  0xBC                    @ the rest of the 0xC0-byte header, left blank
reset:
    mov     r0, #0x12               @ IRQ mode stack
    msr     cpsr_c, r0
    ldr     sp, =0x03007FA0
    mov     r0, #0x1F               @ system mode stack
    msr     cpsr_c, r0
    ldr     sp, =0x03007F00
    ldr     r0, =__iwram_lma        @ copy .iwram from ROM
    ldr     r1, =__iwram_start
    ldr     r2, =__iwram_end
1:  cmp     r1, r2
    ldrlo   r3, [r0], #4
    strlo   r3, [r1], #4
    blo     1b
    ldr     r1, =__bss_start        @ clear .bss
    ldr     r2, =__bss_end
    mov     r3, #0
2:  cmp     r1, r2
    strlo   r3, [r1], #4
    blo     2b
    ldr     r0, =main
    mov     lr, pc
    bx      r0
3:  b       3b
    .ltorg
    .size _start, . - _start
