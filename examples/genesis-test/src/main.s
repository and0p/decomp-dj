; SPDX-License-Identifier: MIT
; The Genesis test ROM: a tiny Mega Drive program for trying the Genesis Plus GX connector
; and the 68000 decoder. It draws nothing but a background colour; each button runs
; different code (README.md). Assembled by tools/asm68k.py.

        .equ VDP_DATA,$C00000
        .equ VDP_CTRL,$C00004
        .equ PAD1_DATA,$A10003
        .equ PAD1_CTRL,$A10009
        .equ VERSION,$A10001
        .equ TMSS,$A14000

; Work RAM.
        .equ frames,$FF0000         ; .l  vertical interrupts so far
        .equ pad,$FF0004            ; .w  buttons held this frame (1 = held)
        .equ pressed,$FF0006        ; .w  buttons pressed since last frame
        .equ colour,$FF0008         ; .w  the background colour
        .equ a_count,$FF000A        ; .w  times A was pressed
        .equ sum,$FF000C            ; .l  what on_b adds up
        .equ mix,$FF0010            ; .w  what ram_mix leaves
        .equ x_count,$FF0012        ; .w  times X was pressed
        .equ RAM_CODE,$FF8000       ; ram_mix is copied here

; Buttons, as read_pad returns them.
        .equ BTN_UP,0
        .equ BTN_DOWN,1
        .equ BTN_LEFT,2
        .equ BTN_RIGHT,3
        .equ BTN_B,4
        .equ BTN_C,5
        .equ BTN_A,6
        .equ BTN_START,7
        .equ BTN_Z,8
        .equ BTN_Y,9
        .equ BTN_X,10
        .equ BTN_MODE,11

; ---- vectors and header -------------------------------------------------------------

        .org 0
        .dc.l $00FFFE00, start
        .dc.l exception, exception, exception, exception, exception, exception
        .dc.l exception, exception, exception, exception, exception, exception
        .dc.l exception, exception, exception, exception, exception, exception
        .dc.l exception, exception, exception, exception, exception, exception
        .dc.l exception, exception, exception, exception
        .dc.l vblank, exception                       ; level 6 (vertical), level 7
        .dc.l exception, exception, exception, exception, exception, exception
        .dc.l exception, exception, exception, exception, exception, exception
        .dc.l exception, exception, exception, exception, exception, exception
        .dc.l exception, exception, exception, exception, exception, exception
        .dc.l exception, exception, exception, exception, exception, exception
        .dc.l exception, exception

        .org $100
        .dc.b "SEGA MEGA DRIVE "
        .dc.b "(C)DJ  2026.NOV "
        .dc.b "DECOMP-DJ GENESIS TEST                          "
        .dc.b "DECOMP-DJ GENESIS TEST                          "
        .dc.b "GM 00000000-00"
        .dc.w 0                                       ; checksum (asm68k.py fills it in)
        .dc.b "J6              "                      ; 3- and 6-button pads
        .dc.l 0, ROM_SIZE-1
        .dc.l $FF0000, $FFFFFF
        .dc.b "            "                          ; no save RAM
        .dc.b "            "
        .dc.b "                                        "
        .dc.b "JUE             "

; ---- start-up -------------------------------------------------------------------------

        .org $200
        .func start
        move.w #$2700,sr
        ; Unlock the VDP on consoles with the TMSS.
        move.b VERSION,d0
        andi.b #$0F,d0
        beq.s .no_tmss
        move.l #'SEGA',TMSS
.no_tmss:
        lea vdp_registers(pc),a0
        moveq #(vdp_registers_end-vdp_registers)/2-1,d1
.vdp:   move.w (a0)+,VDP_CTRL
        dbra d1,.vdp
        ; Clear work RAM.
        lea $FF0000,a0
        move.w #$3FFF,d1
.clear: clr.l (a0)+
        dbra d1,.clear
        ; Copy ram_mix into RAM, where it runs.
        lea ram_mix(pc),a0
        lea RAM_CODE,a1
        move.w #(ram_mix_end-ram_mix)/2-1,d1
.copy:  move.w (a0)+,(a1)+
        dbra d1,.copy
        ; The pad's TH line is an output.
        move.b #$40,PAD1_CTRL
        move.b #$40,PAD1_DATA
        move.w #$0E00,colour
        move.w #$2000,sr                ; interrupts on
        bra.w main
        .endfunc

vdp_registers:
        .dc.w $8004, $8174, $8F02, $8700, $8C81, $9001
vdp_registers_end:

        .func exception
        rte
        .endfunc

        .func vblank
        addq.l #1,frames
        rte
        .endfunc

; ---- the frame loop -------------------------------------------------------------------

        .func main
.loop:  bsr.w wait_vblank
        bsr.w read_pad
        move.w pad,d1
        not.w d1
        and.w d0,d1                     ; held now and not last frame
        move.w d1,pressed
        move.w d0,pad
        ; The handlers may change any register: d0 and d1 are read again after each.
        btst #BTN_A,d1
        beq.s .no_a
        bsr.w on_a
.no_a:  move.w pad,d0
        btst #BTN_B,d0                  ; B: every frame it's held
        beq.s .no_b
        bsr.w on_b
.no_b:  move.w pressed,d1
        btst #BTN_C,d1
        beq.s .no_c
        bsr.w on_c
.no_c:  move.w pressed,d1
        btst #BTN_START,d1
        beq.s .no_start
        jsr on_start
.no_start:
        move.w pressed,d1
        btst #BTN_X,d1
        beq.s .no_x
        bsr.w on_x
.no_x:  move.w pad,d0
        andi.w #$000C,d0                ; left or right
        beq.s .draw
        bsr.w on_direction
.draw:  bsr.w set_colour
        bra.s .loop
        .endfunc

        .func wait_vblank
        move.l frames,d0
.wait:  cmp.l frames,d0
        beq.s .wait
        rts
        .endfunc

; The pad's buttons in d0 (bit set = held), with X, Y, Z and Mode from a 6-button pad.
        .func read_pad
        movem.l d1-d3/a0,-(sp)
        lea PAD1_DATA,a0
        move.b #$40,(a0)                ; TH high: C B Right Left Down Up
        nop
        nop
        move.b (a0),d0
        move.b #$00,(a0)                ; TH low: Start A 0 0 Down Up
        nop
        nop
        move.b (a0),d1
        andi.w #$003F,d0
        andi.w #$0030,d1
        lsl.w #2,d1
        or.w d1,d0
        ; A 6-button pad answers the third TH low with 0000, then X Y Z Mode.
        move.b #$40,(a0)
        nop
        move.b #$00,(a0)
        nop
        move.b #$40,(a0)
        nop
        move.b #$00,(a0)
        nop
        nop
        move.b (a0),d2
        andi.b #$0F,d2
        bne.s .three
        move.b #$40,(a0)
        nop
        nop
        move.b (a0),d2
        not.b d2
        andi.w #$000F,d2
        lsl.w #8,d2
        not.b d0
        andi.w #$00FF,d0
        or.w d2,d0
        bra.s .done
.three: not.b d0
        andi.w #$00FF,d0
.done:  move.b #$40,(a0)
        movem.l (sp)+,d1-d3/a0
        rts
        .endfunc

; Writes the background colour into colour 0.
        .func set_colour
        move.l #$C0000000,VDP_CTRL
        move.w colour,VDP_DATA
        rts
        .endfunc

; ---- what the buttons run -------------------------------------------------------------

; A: one of four cases in turn, through a jump table of branches.
        .func on_a
        move.w a_count,d0
        addq.w #1,a_count
        andi.w #3,d0
        add.w d0,d0
        add.w d0,d0
        jmp .table(pc,d0.w)
.table: bra.w .red
        bra.w .green
        bra.w .blue
        bra.w .white
.red:   move.w #$000E,colour
        rts
.green: move.w #$00E0,colour
        rts
.blue:  move.w #$0E00,colour
        rts
.white: move.w #$0EEE,colour
        rts
        .endfunc

; B: adds up a table, every frame it's held.
        .func on_b
        lea squares(pc),a0
        moveq #0,d1
        moveq #7,d2
.add:   move.w (a0)+,d0
        ext.l d0
        add.l d0,d1
        dbra d2,.add
        add.l d1,sum
        rts
        .endfunc

squares:
        .dc.w 0, 1, 4, 9, 16, 25, 36, 49

; C: calls through every kind of call, and reads a word kept after its code.
        .func on_c
        jsr shade
        jsr shade(pc)
        lea shade,a1
        jsr (a1)
        move.w .step(pc),d0
        add.w d0,colour
        rts
.step:  .dc.w $0002
        .endfunc

        .func shade
        move.w colour,d0
        lsr.w #1,d0
        andi.w #$0EEE,d0
        move.w d0,colour
        rts
        .endfunc

; Start: no segment presses it, so it stays uncovered.
        .func on_start
        clr.w colour
        clr.l sum
        rts
        .endfunc

; X (a 6-button pad only): multiplies.
        .func on_x
        addq.w #1,x_count
        move.w x_count,d0
        mulu #$0111,d0
        andi.w #$0EEE,d0
        move.w d0,colour
        rts
        .endfunc

; Left or Right: calls ram_mix, which runs from work RAM.
        .func on_direction
        move.w pad,d0
        jsr RAM_CODE
        move.w d0,mix
        add.w d0,colour
        rts
        .endfunc

; Copied to RAM at start-up and run there (coverage flags it in the RAM region; it rolls
; up only ROM, so as a function it shows as never entered).
        .func ram_mix
        swap d0
        clr.w d0
        swap d0
        lsl.w #4,d0
        eori.w #$0123,d0
        addq.w #2,d0
        rts
        .endfunc
ram_mix_end:
