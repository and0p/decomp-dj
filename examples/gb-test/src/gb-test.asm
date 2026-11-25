; The Game Boy test ROM for decomp-dj's SameBoy connector: a few functions in four ROM
; banks, one copied to HRAM, each run by a different button. Built by ../build-rom.py,
; which also writes the cartridge header (MBC5, 8 KB of battery RAM, Color compatible).

frames   equ $C000      ; frames since power-on, counted by hram_tick (16 bits)
joy      equ $C002      ; buttons held: A, B, Select, Start, Right, Left, Up, Down from bit 0
a_count  equ $C003      ; times A was seen held
score    equ $C004
x_pos    equ $C005
banked   equ $D000      ; in WRAM bank 3 on a Game Boy Color (bank 1 on a Game Boy)
saved    equ $A000      ; in the cartridge's battery RAM: times B was pressed

; ---- bank 0 -----------------------------------------------------------------------------
bank 0
org $0100
    nop                 ; the cartridge's entry point, then its header (filled in by the
    jp start            ; build script)

org $0150
proc start
    di
    ld sp, $E000
    ; The per-frame routine runs from HRAM: copy it there.
    ld hl, hram_code
    ld c, $80
    ld b, hram_code_end - hram_code
.copy:
    ld a, [hl+]
    ldh [c], a
    inc c
    dec b
    jr nz, .copy
    ; Cartridge RAM on, for the battery-backed counter.
    ld a, $0A
    ld [$0000], a
    xor a
    ld [frames], a
    ld [frames + 1], a
    ld [a_count], a
    ld [score], a
    ld a, $50
    ld [x_pos], a
main_loop:              ; once a frame: the project's logic tick
    call wait_vblank
    call hram_tick
    call read_joypad
    ld [joy], a
    bit 0, a
    call nz, press_a
    ld a, [joy]
    bit 1, a
    call nz, press_b
    ld a, [joy]
    bit 3, a
    call nz, press_start
    ld a, [joy]
    and $30
    call nz, on_direction
    jr main_loop
endproc

; Waits for the start of the next vertical blank (LY reaching 144).
proc wait_vblank
.out:
    ldh a, [$FF44]
    cp 144
    jr z, .out
.in:
    ldh a, [$FF44]
    cp 144
    jr nz, .in
    ret
endproc

; Returns the buttons held in a: A, B, Select, Start in bits 0-3, Right, Left, Up, Down in
; bits 4-7.
proc read_joypad
    ld a, $20           ; the d-pad
    ldh [$FF00], a
    ldh a, [$FF00]
    ldh a, [$FF00]
    cpl
    and $0F
    swap a
    ld b, a
    ld a, $10           ; the buttons
    ldh [$FF00], a
    ldh a, [$FF00]
    ldh a, [$FF00]
    ldh a, [$FF00]
    cpl
    and $0F
    or b
    ld b, a
    ld a, $30
    ldh [$FF00], a
    ld a, b
    ret
endproc

proc press_a
    ld a, 1
    ld [$2100], a       ; ROM bank 1 at $4000
    call on_a
    ret
endproc

proc press_b
    ld a, 2
    ld [$2100], a
    call on_b
    ret
endproc

proc press_start
    ld a, 3
    ld [$2100], a
    call on_start
    ret
endproc

; Right or Left (a holds the d-pad bits): through a jump table.
proc on_direction
    swap a
    and $03             ; 1 Right, 2 Left, 3 both
    add a, a
    ld e, a
    ld d, 0
    ld hl, dir_table
    add hl, de
    ld a, [hl+]
    ld h, [hl]
    ld l, a
    jp hl
endproc

dir_table:
    dw dir_none, dir_right, dir_left, dir_both

proc dir_none           ; never: on_direction isn't called without a direction
    ret
endproc

proc dir_right
    ld hl, x_pos
    inc [hl]
    ret
endproc

proc dir_left
    ld hl, x_pos
    dec [hl]
    ret
endproc

proc dir_both           ; Right and Left at once: no segment does that
    xor a
    ld [x_pos], a
    ret
endproc

; Copied to HRAM at $FF80 by start, and run there every frame.
hram_code:
load $FF80
hram_tick:
    ld hl, frames
    inc [hl]
    ret nz
    inc hl
    inc [hl]
    ret
endload
hram_code_end:

; ---- bank 1 -----------------------------------------------------------------------------
bank 1
org $4000
proc on_a
    ld hl, a_count
    inc [hl]
    ld a, [hl]
    and $03
    jr nz, .done
    call a_every_fourth
.done:
    ret
endproc

proc a_every_fourth
    ld a, [a_count]
    ld [score], a
    ret
endproc

; ---- bank 2 -----------------------------------------------------------------------------
bank 2
org $4000
proc on_b
    ld a, 3
    ldh [$FF70], a      ; WRAM bank 3 at $D000 (a Game Boy has only bank 1 there)
    ld hl, banked
    inc [hl]
    ld a, 1
    ldh [$FF70], a
    ld hl, saved
    inc [hl]
    ret
endproc

; ---- bank 3 -----------------------------------------------------------------------------
bank 3
org $4000
proc on_start           ; Start: no segment presses it, so it stays uncovered
    ld hl, score
    ld [hl], 0
    ret
endproc
