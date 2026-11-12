; pattern2.asm - SheikhEmu self-made M2 verification cartridge #2.

; 100% original content written for this project: no copyrighted data,
; no third-party code, no trademarked assets. Its purpose is the M2
; INDEPENDENT frame verification (category A): a deterministic static
; scene with strongly ASYMMETRIC markers (a per-column ruler, arrows,
; letters) that exposes any wrong scroll direction, flip or
; line-granular split, exercised through the background pipeline, the
; sprite pipeline, the sprite-0 hit and a line-granular mid-frame
; scroll split (status-bar style raster effect).
;
; Layout: NROM (mapper 0), 16 KiB PRG at $C000-$FFFF, CHR RAM 8 KiB
; (the program uploads its own 48 tiles through $2006/$2007 at
; setup). Vertical (side-by-side) mirroring: nametable B next to A.
;
; Scene (the spec the independent model expects):
;   * rows 0 and 12: ruler glyphs (tile column+2) - one DISTINCT
;     glyph per nametable column, so a horizontal shift of ANY size
;     is readable directly off the frame;
;   * rows 1-11 and 13-29: solid tile 1 except column 31, which holds
;     a vertical ruler (tile 2 + row/4: a different glyph per quarter
;     of the screen height, exposing vertical shifts);
;   * row 8: the letters S P L I T at columns 8-12 (asymmetric
;     letterforms; sprite 2 shows through their transparent pixels);
;   * sprite 0 (solid, the hit source) at x=64 y=100: hit fires on
;     line 101 over the tear row's checker glyph;
;   * sprite 1 (right-arrow) at x=120 y=140: direction marker;
;   * sprite 2 (solid, BEHIND the background) at x=80 y=63: visible
;     only through the letters' transparent pixels on lines 64-71;
;   * the split: rows 0..101 render at scroll X=0, rows 102..239 at
;     scroll X=128 (the right half of the lower screen comes from the
;     never-written nametable B = backdrop).
;
; The sprite-0 poll uses the hardware-correct ORDER (this is the fix
; of the documented assembly defect of the lost first version):
; first spin until the stale hit flag clears (the pre-render line
; clears it at dot 1), THEN wait for this frame's hit. Polling for
; the flag directly after the vblank scroll reset would escape
; during vblank on the previous frame's still-set flag and fire the
; split before the frame starts (whole-frame tearing).
;
; Observable state: zero page $10 counts frames, $11 counts splits.
; The rendered frame itself is the verification evidence.

    .org $C000

reset:
    sei                 ; no interrupts during setup
    cld                 ; binary arithmetic (no decimal mode on the 2A03)
    ldx #$FF
    txs                 ; a known stack

    ; ---- warm-up: two vblanks before touching VRAM -----------------
wait1:
    bit $2002
    bpl wait1
wait2:
    bit $2002
    bpl wait2

    ; ---- upload 48 tiles (768 bytes) into CHR RAM at $0000 ----------
    lda #$00
    sta $2006
    sta $2006           ; v = $0000
    lda #$00
    sta $10             ; source pointer lo: tiles at $D000
    lda #$D0
    sta $11             ; source pointer hi
    ldx #$03            ; three 256-byte pages
chr_page:
    ldy #$00
chr_byte:
    lda ($10),y
    sta $2007
    iny
    bne chr_byte
    inc $11
    dex
    bne chr_page

    ; ---- palette: 32 bytes from $D400 -------------------------------
    bit $2002           ; reset the write toggle
    lda #$3F
    sta $2006
    lda #$00
    sta $2006
    ldx #$00
pal_loop:
    lda pal_data,x
    sta $2007
    inx
    cpx #$20
    bne pal_loop

    ; ---- nametable 0 -------------------------------------------------
    bit $2002
    lda #$20
    sta $2006
    lda #$00
    sta $2006           ; v = $2000
    ldy #$00            ; row counter 0..29
nt_row:
    cpy #$00
    beq ruler_row       ; row 0: ruler glyphs
    cpy #$0C
    beq ruler_row       ; row 12: ruler glyphs again (the tear row)
    ldx #$00            ; normal rows
nt_col:
    cpx #$1F
    beq col31           ; column 31: vertical-ruler glyph
    cpy #$08
    bne plain_tile      ; not the letter row
    cpx #$08
    bcc plain_tile      ; letters only at columns 8..12
    cpx #$0D
    bcc letter_tile
    jmp plain_tile
letter_tile:
    txa
    sec
    sbc #$08            ; 0..4 within the letter run
    clc
    adc #$22            ; tiles $22..$26 = 34..38 (S P L I T)
    sta $2007
    inx
    cpx #$20
    bne nt_col
    jmp row_done
plain_tile:
    lda #$01
    sta $2007
    inx
    cpx #$20
    bne nt_col
    jmp row_done
col31:
    tya
    lsr a
    lsr a               ; row/4 (0..7)
    clc
    adc #$02            ; tile 2..9 per screen quarter
    sta $2007
row_done:
    iny
    cpy #$1E
    bne nt_row
    beq nt_done         ; 30 rows written: leave the row loop

ruler_row:
    ldx #$00
ruler_col:
    txa
    clc
    adc #$02            ; tile 2..33: one distinct glyph per column
    sta $2007
    inx
    cpx #$20
    bne ruler_col
    iny
    cpy #$1E
    bne nt_row
    ; y reached 30 here too: fall through to the attributes
nt_done:

    ; ---- attributes: 64 bytes, all $00 (palette 0 everywhere) -------
    ldx #$00
    lda #$00
attr_loop:
    sta $2007
    inx
    cpx #$40
    bne attr_loop

    ; ---- OAM shadow page $0200 ---------------------------------------
    ; Default: y=$F8 (off screen), zeroed tile/attr/x.
    ldx #$00
oam_fill:
    txa
    and #$03
    bne oam_zero
    lda #$F8            ; entry byte 0: y off-screen
    jmp oam_store
oam_zero:
    lda #$00
oam_store:
    sta $0200,x
    inx
    bne oam_fill

    ; Sprite 0: the hit source. y=100 -> lines 101..108, x=64,
    ; solid tile 1, palette 0, in front of the background.
    lda #$64
    sta $0200           ; y
    lda #$01
    sta $0201           ; tile (solid)
    lda #$00
    sta $0202           ; attr
    lda #$40
    sta $0203           ; x

    ; Sprite 1: right-arrow glyph at x=120, y=140, in front.
    lda #$8C
    sta $0204           ; y
    lda #$0D
    sta $0205           ; tile 13 (arrowR)
    lda #$00
    sta $0206
    lda #$78
    sta $0207           ; x

    ; Sprite 2: solid, BEHIND the background, at x=80, y=63 -> lines
    ; 64..71: shows only through the letters' transparent pixels.
    lda #$3F
    sta $0208           ; y
    lda #$01
    sta $0209           ; tile (solid)
    lda #$21
    sta $020A           ; palette 1, behind background
    lda #$50
    sta $020B           ; x

    ; ---- scroll zero, OAM DMA, rendering on ---------------------------
    bit $2002
    lda #$00
    sta $2005           ; X = 0
    sta $2005           ; Y = 0
    lda #$00
    sta $2006
    sta $2006           ; v = t = nametable 0, no scroll

    lda #$02
    sta $4014           ; OAM DMA from the shadow page

    lda #$00
    sta $10             ; frame counter
    sta $11             ; split counter

    lda #$1E            ; bg + sprites, both left-8 shown
    sta $2001
    lda #$00
    sta $2000           ; NMI off: the polls drive the flow

main:
    ; Wait for vblank (flag poll).
    bit $2002
    bpl main
    inc $10             ; a frame completed

    ; Reset the scroll for the top of the coming frame (the split of
    ; the previous frame left X=128 in t).
    lda #$00
    sta $2005           ; X = 0
    sta $2005           ; Y = 0
    lda #$00
    sta $2006
    sta $2006           ; t = nametable 0, no scroll

    ; Hardware-correct sprite-0 poll: first let the stale flag clear.
wait_clear:
    lda $2002
    and #$40
    bne wait_clear
hit_wait:
    lda $2002
    and #$40
    beq hit_wait        ; this frame's hit (line 101, x=64)

    ; The split: the lower screen scrolls X=128. The write lands
    ; ~45 dots after the hit - before the dot-320 snapshot of line
    ; 101 - so rows 0..101 render at X=0 and rows 102..239 at X=128.
    lda #$80
    sta $2005           ; X = 128 (coarse 16, fine 0)
    lda #$00
    sta $2005           ; Y unchanged

    inc $11             ; the split executed
    jmp main

nmi:
    rti

irq:
    rti

; ---- tile data (uploaded to CHR RAM) --------------------------------
; 48 tiles of 16 bytes at $D000. Tiles 39..47 are blank filler.
    .org $D000
tiles:

    ; tile 0 (0x0000 in the table)
    .db $00, $00, $00, $00, $00, $00, $00, $00; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    .db $FF, $FF, $FF, $FF, $FF, $FF, $FF, $FF; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    ; tile 2 (0x0020 in the table)
    .db $FF, $FF, $C3, $C3, $C3, $C3, $FF, $FF; lo
    .db $00, $7E, $42, $42, $42, $42, $7E, $00; hi
    .db $80, $80, $80, $80, $80, $80, $80, $80; lo
    .db $00, $60, $00, $60, $00, $60, $00, $60; hi
    ; tile 4 (0x0040 in the table)
    .db $FF, $00, $00, $00, $00, $00, $00, $00; lo
    .db $00, $FF, $00, $00, $00, $00, $00, $00; hi
    .db $00, $00, $00, $00, $00, $00, $00, $FF; lo
    .db $00, $00, $00, $00, $00, $00, $FF, $00; hi
    ; tile 6 (0x0060 in the table)
    .db $80, $40, $20, $10, $08, $04, $02, $01; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    .db $01, $02, $04, $08, $10, $20, $40, $80; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    ; tile 8 (0x0080 in the table)
    .db $10, $00, $10, $00, $10, $00, $10, $00; lo
    .db $00, $10, $00, $10, $00, $10, $00, $10; hi
    .db $00, $00, $00, $FF, $00, $00, $00, $00; lo
    .db $00, $00, $00, $00, $FF, $00, $00, $00; hi
    ; tile 10 (0x00a0 in the table)
    .db $AA, $55, $AA, $55, $AA, $55, $AA, $55; lo
    .db $55, $AA, $55, $AA, $55, $AA, $55, $AA; hi
    .db $F0, $F0, $F0, $F0, $F0, $F0, $F0, $F0; lo
    .db $F0, $F0, $F0, $F0, $F0, $F0, $F0, $F0; hi
    ; tile 12 (0x00c0 in the table)
    .db $0F, $0F, $0F, $0F, $0F, $0F, $0F, $0F; lo
    .db $0F, $0F, $0F, $0F, $0F, $0F, $0F, $0F; hi
    .db $00, $00, $00, $00, $00, $00, $00, $00; lo
    .db $10, $08, $FC, $08, $10, $00, $00, $00; hi
    ; tile 14 (0x00e0 in the table)
    .db $08, $10, $3E, $10, $08, $00, $00, $00; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    .db $18, $3C, $7E, $18, $18, $18, $00, $00; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    ; tile 16 (0x0100 in the table)
    .db $00, $18, $18, $18, $7E, $3C, $18, $00; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    .db $81, $42, $24, $18, $18, $24, $42, $81; lo
    .db $81, $42, $24, $18, $18, $24, $42, $81; hi
    ; tile 18 (0x0120 in the table)
    .db $00, $10, $10, $FE, $10, $10, $10, $00; lo
    .db $80, $00, $00, $00, $00, $00, $00, $00; hi
    .db $3C, $66, $42, $C1, $C1, $42, $66, $3C; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    ; tile 20 (0x0140 in the table)
    .db $FE, $FE, $FE, $FE, $00, $00, $00, $00; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    .db $00, $00, $00, $00, $00, $00, $00, $00; lo
    .db $00, $00, $00, $00, $7E, $7E, $7E, $7E; hi
    ; tile 22 (0x0160 in the table)
    .db $FF, $81, $81, $81, $81, $81, $81, $FF; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    .db $C3, $C3, $00, $00, $00, $00, $00, $00; lo
    .db $00, $00, $00, $00, $00, $00, $C3, $C3; hi
    ; tile 24 (0x0180 in the table)
    .db $40, $40, $40, $40, $40, $40, $40, $40; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    .db $88, $88, $88, $88, $04, $04, $04, $04; lo
    .db $00, $00, $00, $00, $80, $80, $80, $80; hi
    ; tile 26 (0x01a0 in the table)
    .db $FE, $00, $00, $00, $00, $00, $00, $00; lo
    .db $00, $00, $00, $FE, $00, $00, $00, $00; hi
    .db $80, $C0, $E0, $F0, $F8, $FC, $FE, $FF; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    ; tile 28 (0x01c0 in the table)
    .db $00, $00, $00, $00, $00, $00, $00, $00; lo
    .db $01, $03, $07, $0F, $1F, $3F, $7F, $FF; hi
    .db $FE, $18, $18, $18, $18, $18, $18, $00; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    ; tile 30 (0x01e0 in the table)
    .db $00, $18, $18, $18, $18, $18, $18, $FE; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    .db $81, $00, $00, $00, $00, $00, $00, $81; lo
    .db $00, $00, $18, $00, $00, $18, $00, $00; hi
    ; tile 32 (0x0200 in the table)
    .db $81, $81, $81, $FF, $81, $81, $81, $81; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    .db $80, $40, $20, $40, $80, $40, $20, $40; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    ; tile 34 (0x0220 in the table)
    .db $00, $00, $00, $00, $00, $00, $00, $00; lo
    .db $78, $60, $60, $78, $0C, $0C, $78, $00; hi
    .db $00, $00, $00, $00, $00, $00, $00, $00; lo
    .db $78, $6C, $6C, $78, $60, $60, $60, $00; hi
    ; tile 36 (0x0240 in the table)
    .db $00, $00, $00, $00, $00, $00, $00, $00; lo
    .db $60, $60, $60, $60, $60, $60, $7C, $00; hi
    .db $00, $00, $00, $00, $00, $00, $00, $00; lo
    .db $7C, $18, $18, $18, $18, $18, $18, $00; hi
    ; tile 38 (0x0260 in the table)
    .db $00, $00, $00, $00, $00, $00, $00, $00; lo
    .db $7C, $18, $18, $18, $18, $18, $18, $00; hi
    .db $00, $00, $00, $00, $00, $00, $00, $00; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    ; tile 40 (0x0280 in the table)
    .db $00, $00, $00, $00, $00, $00, $00, $00; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    .db $00, $00, $00, $00, $00, $00, $00, $00; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    ; tile 42 (0x02a0 in the table)
    .db $00, $00, $00, $00, $00, $00, $00, $00; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    .db $00, $00, $00, $00, $00, $00, $00, $00; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    ; tile 44 (0x02c0 in the table)
    .db $00, $00, $00, $00, $00, $00, $00, $00; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    .db $00, $00, $00, $00, $00, $00, $00, $00; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    ; tile 46 (0x02e0 in the table)
    .db $00, $00, $00, $00, $00, $00, $00, $00; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi
    .db $00, $00, $00, $00, $00, $00, $00, $00; lo
    .db $00, $00, $00, $00, $00, $00, $00, $00; hi

; ---- palette data (32 bytes) ----------------------------------------
    .org $D400
pal_data:
    .db $0F, $30, $16, $01
    .db $0F, $30, $16, $01
    .db $0F, $30, $16, $01
    .db $0F, $30, $16, $01
    .db $0F, $30, $16, $01
    .db $0F, $27, $17, $07
    .db $0F, $29, $19, $09
    .db $0F, $24, $14, $04

; Interrupt vectors close the 16 KiB bank: NMI, RESET, IRQ.
    .org $FFFA
    .dw nmi, reset, irq

