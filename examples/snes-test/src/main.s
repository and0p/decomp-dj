; SNES test ROM for decomp-dj: a tiny program for trying the Mesen-S connector and the 65816
; decoder end to end. It draws nothing but the backdrop, whose colour changes; each button
; runs different code. Built with ca65 (cc65) as LoROM, or as HiROM with -D HIROM.
; MIT, like the rest of decomp-dj.

.p816
.smart -

INIDISP = $2100
CGADD   = $2121
CGDATA  = $2122
NMITIMEN = $4200
RDNMI   = $4210
HVBJOY  = $4212
JOY1    = $4218

; Pad bits in JOY1 (16 bits)
PAD_B     = $8000
PAD_START = $1000
PAD_LEFT  = $0200
PAD_RIGHT = $0100
PAD_A     = $0080

.segment "ZEROPAGE"
frames:   .res 2          ; NMIs so far
joy:      .res 2          ; this frame's pad
color:    .res 2          ; the backdrop colour, BGR555
presses:  .res 2          ; times B was seen down

; The routine that runs from WRAM: placed in ROM, copied to WRAM at start-up (ld65 gives
; where it is in each).
.import __RAMCODE_LOAD__: abs, __RAMCODE_RUN__: abs, __RAMCODE_SIZE__: abs

.segment "CODE"

; Power-on: emulation mode, 8-bit registers.
.proc reset
	sei
	clc
	xce                   ; native mode
	rep #$10              ; 16-bit X and Y
	.i16
	ldx #$1FFF
	txs
	rep #$20
	.a16
	lda #$0000
	tcd                   ; direct page at $0000
	stz frames
	stz joy
	stz presses
	lda #$001F
	sta color             ; red
	; Copy the WRAM routine into place with a block move (three-byte MVN).
	ldx #.loword(__RAMCODE_LOAD__)
	ldy #.loword(__RAMCODE_RUN__)
	lda #__RAMCODE_SIZE__ - 1
	mvn #^__RAMCODE_LOAD__, #^__RAMCODE_RUN__
	phk
	plb                   ; data bank: this one again (MVN changed it)
	sep #$20
	.a8
	lda #$0F
	sta INIDISP           ; screen on, full brightness
	lda #$81
	sta NMITIMEN          ; NMI and the automatic pad read
	cli
loop:
	wai
	jsr frame
	bra loop
.endproc

; Once a frame, after the NMI: read the pad and act on it. 8-bit A, 16-bit X and Y.
.proc frame
	.a8
	.i16
wait:
	lda HVBJOY
	and #$01
	bne wait              ; the pad is still being read
	rep #$20
	.a16
	lda JOY1
	sta joy
	bit #PAD_A
	beq :+
	jsl on_a
:	lda joy
	bit #PAD_B
	beq :+
	jsr on_b
:	lda joy
	bit #PAD_START
	beq :+
	jsr on_start
:	lda joy
	and #PAD_LEFT | PAD_RIGHT
	beq :+
	jsl ram_routine       ; runs from WRAM
:	sep #$20
	.a8
	jsr show
	rts
.endproc

; Writes the colour to CGRAM entry 0, the backdrop. 8-bit A.
.proc show
	.a8
	stz CGADD
	lda color
	sta CGDATA
	lda color + 1
	sta CGDATA
	rts
.endproc

; B: picks one of four colour changes through a jump table, by how many times B was seen.
; 16-bit A.
.proc on_b
	.a16
	inc presses
	lda presses
	and #$0003
	asl a
	tax
	jmp (b_table, x)
b0:	lda #$7C00            ; blue
	bra done
b1:	lda #$03E0            ; green
	bra done
b2:	lda #$7FFF            ; white
	bra done
b3:	lda color
	eor #$7FFF            ; inverse
done:
	sta color
	; Save it in the cartridge's save RAM ($70:0000 on LoROM, $30:6000 on HiROM).
	sep #$20
	.a8
	lda color
	sta f:SRAM_COLOR
	rep #$20
	.a16
	rts
.endproc

; Start: never pressed by the example's segments, so it stays uncovered.
.proc on_start
	.a16
	lda #$0000
	sta color
	rts
.endproc

; The NMI (native mode): count frames and acknowledge it.
.proc nmi
	rep #$30
	.a16
	.i16
	pha
	inc frames
	sep #$20
	.a8
	lda RDNMI
	rep #$20
	.a16
	pla
	rti
.endproc

; Unused interrupts.
.proc unused
	rti
.endproc

.segment "RODATA"
b_table:
	.addr on_b::b0, on_b::b1, on_b::b2, on_b::b3

; Code in the second 32 KB bank (LoROM: $01:8000; HiROM: $C0:0000). Called with JSL.
.segment "CODE1"

; A: brightens the colour, in 16-bit A, with a 16-bit immediate.
.proc on_a
	php
	rep #$20
	.a16
	lda color
	clc
	adc #$0421
	and #$7FFF
	sta color
	jsr fade
	plp
	rtl
.endproc

; Called from on_a in the same bank (JSR, RTS). 8-bit A inside.
.proc fade
	php
	sep #$20
	.a8
	lda frames
	and #$07
	bne :+
	lda #$0E
	sta INIDISP           ; a touch darker every eighth frame
:	plp
	rts
.endproc

; The routine copied to WRAM at start-up and run there (Left and Right). Called with JSL
; in 16-bit A.
.segment "RAMCODE"
.proc ram_routine
	.a16
	lda color
	xba
	and #$7FFF
	sta color
	rtl
.endproc

.segment "HEADER"
	.byte "DJ SNES TEST         "  ; 21 bytes
.ifdef HIROM
	.byte $21                     ; HiROM
.else
	.byte $20                     ; LoROM
.endif
	.byte $02                     ; ROM, RAM and battery
	.byte $06                     ; 64 KB of ROM
	.byte $01                     ; 2 KB of SRAM
	.byte $01                     ; North America
	.byte $00                     ; developer
	.byte $00                     ; version
	.word $FFFF, $0000            ; checksum complement and checksum (build-rom.sh fills them in)

.segment "VECTORS"
	; Native mode
	.word 0, 0
	.addr unused                  ; COP
	.addr unused                  ; BRK
	.addr unused                  ; ABORT
	.addr nmi                     ; NMI
	.word 0
	.addr unused                  ; IRQ
	; Emulation mode
	.word 0, 0
	.addr unused                  ; COP
	.word 0
	.addr unused                  ; ABORT
	.addr unused                  ; NMI
	.addr reset                   ; RESET
	.addr unused                  ; IRQ and BRK

.ifdef HIROM
SRAM_COLOR = $306000
.else
SRAM_COLOR = $700000
.endif
