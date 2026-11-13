; banktest_cnrom.asm — CNROM (iNES mapper 3) CHR bank verification.
;
; PRG is fixed 32 KiB (the engine itself); only the 8 KiB CHR window
; switches (value bits 0-1). The test writes each selector to the
; register at $8000, then reads the CHR signature bytes through
; $2007 at window offset $0000 and $1FFF.

.org $8000
.equ WSTAT, $6000
.equ WDE,   $6001
.equ WBO,   $6002
.equ WTXT,  $6004

start:
    lda #$80                ; status: running
    sta WSTAT
    lda #$DE
    sta WDE
    lda #$B0
    sta WBO
    ldx #$00                ; test index 0..3
loop:
    lda sel,x
    sta $8000               ; CNROM CHR bank register
    lda #$00
    sta $2006
    sta $2006               ; PPU address -> $0000
    lda $2007               ; discard the buffered read
    lda $2007               ; CHR at window offset $0000
    cmp chrlo,x
    bne fail
    lda #$1F
    sta $2006
    lda #$FF                ; PPU address -> $1FFF
    sta $2006
    lda $2007               ; discard
    lda $2007               ; CHR at window offset $1FFF
    cmp chrhi,x
    bne fail
    inx
    cpx #$04
    bne loop
    lda #$00                ; status: all tests passed
    sta WSTAT
    ldy #$00
wok:
    lda txt_ok,y
    beq spin
    sta WTXT,y
    iny
    bne wok
spin:
    jmp spin
fail:
    inx                     ; 1-based failing test number
    stx WSTAT
    ldy #$00
wbad:
    lda txt_bad,y
    beq spin
    sta WTXT,y
    iny
    bne wbad

; Selectors and expected CHR signatures (banks 0..3 of four).
sel:     .db $00, $01, $02, $03
chrlo:   .db $C0, $C1, $C2, $C3
chrhi:   .db $C4, $C5, $C6, $C7
txt_ok:  .db "CNROM OK", 0
txt_bad: .db "CNROM FAIL", 0

.org $FFFA
    .dw spin, start, spin
