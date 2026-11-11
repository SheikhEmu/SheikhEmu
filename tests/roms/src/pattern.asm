; pattern.asm - SheikhEmu self-made M2 calibration cartridge.
;
; 100% original content written for this project: no copyrighted data,
; no third-party code, no trademarked assets. Its purpose is the M2
; "frame 240p" calibration (category A): a deterministic static scene
; that exercises the background pipeline with fine scrolling, the
; sprite pipeline through OAM DMA, the sprite-0 hit, and a line-granular
; mid-frame scroll split (a status-bar style raster effect).
;
; Layout: NROM (mapper 0), 16 KiB PRG at $C000-$FFFF, CHR RAM 8 KiB
; (the program uploads its own tiles through $2006/$2007 at setup).
;
; Observable behavior for the gates:
;   $0200 counts completed frames (the vblank poll loop).
;   $0201 counts executed splits (the sprite-0 poll loop escaped).
;   The rendered frame itself is the golden evidence.

    .org $C000

reset:
    sei                 ; no interrupts during setup
    cld                 ; binary arithmetic (no decimal mode on the 2A03)
    ldx #$FF
    txs                 ; a known stack

    ; Two vblanks: the documented PPU warm-up before touching VRAM.
wait1:
    bit $2002
    bpl wait1
wait2:
    bit $2002
    bpl wait2

    ; ---- upload 32 tiles (512 bytes) into CHR RAM at $0000 -------
    lda #$00
    sta $2006
    sta $2006           ; v = $0000 (two writes toggle w correctly)
    lda #$00
    sta $10             ; source pointer lo: tile data at $D000
    lda #$D0
    sta $11             ; source pointer hi
    ldx #$02            ; two 256-byte pages
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

    ; ---- palette ---------------------------------------------------
    ; Point v at $3F00.
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

    ; ---- nametable 0: a checkerboard of tiles 1..4 -----------------
    ; Point v at $2000.
    bit $2002
    lda #$20
    sta $2006
    lda #$00
    sta $2006
    lda #$00
    sta $12             ; row counter
nt_row:
    ldx #$00
nt_col:
    txa
    eor $12             ; (x xor row) makes the checkerboard
    and #$03
    clc
    adc #$01            ; tiles 1..4 (tile 0 stays blank)
    sta $2007
    inx
    cpx #$20
    bne nt_col
    inc $12
    lda $12
    cmp #$1E
    bne nt_row

    ; Attributes: 64 bytes alternating $00/$55/$AA/$FF quadrants.
    ldx #$00
    lda #$00
attr_loop:
    eor $55             ; $00 -> $55 -> $00 ... every other byte flips
    sta $2007
    inx
    cpx #$40
    bne attr_loop

    ; ---- sprites: fill the OAM shadow page $0200 -------------------
    ; Every sprite slot y=$F0 (off screen) with zeroed bytes; sprite 0
    ; and two decorative sprites are set below.
    ldx #$00
oam_fill:
    txa
    and #$03            ; offset within the 4-byte entry
    bne oam_zero
    lda #$F0            ; entry byte 0: y off-screen
    jmp oam_store
oam_zero:
    lda #$00
oam_store:
    sta $0200,x
    inx
    bne oam_fill

    ; Sprite 0: at x=64, y=100, tile 6 (solid), palette 0, in front.
    lda #$64
    sta $0200           ; y
    lda #$06
    sta $0201           ; tile
    lda #$00
    sta $0202           ; attr
    lda #$40
    sta $0203           ; x

    ; Two decorative sprites in the solid band (behind the bg).
    lda #$E8
    sta $0204           ; y
    lda #$06
    sta $0205
    lda #$23            ; palette 3, behind background
    sta $0206
    lda #$50
    sta $0207

    lda #$E8
    sta $0208
    lda #$06
    sta $0209
    lda #$21
    sta $020A
    lda #$B0
    sta $020B

    ; ---- scroll zero + enable bg/sprites with left columns ---------
    bit $2002
    lda #$00
    sta $2005           ; X = 0
    sta $2005           ; Y = 0
    lda #$00
    sta $2006
    sta $2006           ; v = t = nametable 0, no scroll

    ; Launch the OAM DMA from the shadow page (rendering still off).
    lda #$02
    sta $4014

    lda #$00
    sta $10             ; frame counter (zero page; the setup
                         ; pointers above are finished with it)
    sta $11             ; split counter
    sta $12             ; hit-observed guard

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

    ; Wait for the sprite-0 hit during the visible area.
    lda #$00
    sta $12             ; guard: 0 until the hit is seen
hit_wait:
    lda $2002
    and #$40
    beq hit_wait
    inc $12             ; the hit was observed

    ; The split: move the horizontal scroll of the lower screen to
    ; X=128 (the write pair lands on the next scanline's snapshot).
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

; ---- tile data (uploaded to CHR RAM) -------------------------------
; 32 tiles of 16 bytes each, at $D000.
;   tile 1: solid color 3      tile 2: even pixels color 1
;   tile 3: odd pixels color 2 tile 4: 2-wide vertical stripes
;   tile 5: horizontal stripe  tile 6: solid color 2 (sprite body)
    .org $D000
tiles:
    ; tile 0: blank
    .fill 16, $00
    ; tile 1: solid (both planes $FF)
    .fill 8, $FF
    .fill 8, $FF
    ; tile 2: even pixels index 1 (lo $AA, hi $00)
    .fill 8, $AA
    .fill 8, $00
    ; tile 3: odd pixels index 2 (lo $00, hi $55)
    .fill 8, $00
    .fill 8, $55
    ; tile 4: 2-wide vertical stripes (lo $CC, hi $00)
    .fill 8, $CC
    .fill 8, $00
    ; tile 5: horizontal stripe (rows 0-3 solid, 4-7 blank)
    .fill 4, $FF
    .fill 4, $00
    .fill 4, $FF
    .fill 4, $00
    ; tile 6: solid color 2 for sprites (lo $00, hi $FF)
    .fill 8, $00
    .fill 8, $FF
    ; tiles 7-31: blank
    .fill 400, $00

; ---- palette data --------------------------------------------------
    .org $D400
pal_data:
    ; background: backdrop black, then blue/red/yellow-ish entries.
    .db $0F, $21, $11, $01
    .db $0F, $26, $16, $06
    .db $0F, $2A, $1A, $0A
    .db $0F, $23, $13, $03
    ; sprites: palettes 0-3.
    .db $0F, $27, $17, $07
    .db $0F, $2B, $1B, $0B
    .db $0F, $29, $19, $09
    .db $0F, $24, $14, $04

; Interrupt vectors close the 16 KiB bank: NMI, RESET, IRQ.
    .org $FFFA
    .dw nmi, reset, irq
