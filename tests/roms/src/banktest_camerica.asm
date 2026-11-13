; banktest_camerica.asm — Camerica (iNES mapper 71) bank verification.
;
; The engine lives in the fixed last 16 KiB bank at $C000 (the board
; fixes it there). The bank register listens on writes to
; $C000-$FFFF; writes to $8000-$BFFF must be ignored. The test first
; proves the $8000 write is ignored, then switches each bank through
; a $C000 write and reads the window at $8000/$BFFF.

.org $C000
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
    ; Test 0: a write below $C000 must not move the bank.
    lda #$02
    sta $8000               ; ignored range on this board
    lda $8000
    cmp #$B0                ; power-on bank 0 signature
    bne fail0
    ldx #$00                ; test index 0..2
loop:
    lda sel,x
    sta $C000               ; the Camerica bank register
    lda $8000
    cmp prglo,x
    bne fail
    lda $BFFF
    cmp prghi,x
    bne fail
    inx
    cpx #$03
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
    inx                     ; 1-based failing test number (from the loop)
    stx WSTAT
    jmp wbad
fail0:
    lda #$01                ; test 1 failed (the $8000 write moved the bank)
    sta WSTAT
wbad:
    ldy #$00
wloop:
    lda txt_bad,y
    beq spin
    sta WTXT,y
    iny
    bne wloop

; Selectors and expected signatures (data banks 0..2 of four).
sel:     .db $00, $01, $02
prglo:   .db $B0, $B1, $B2
prghi:   .db $B4, $B5, $B6
txt_ok:  .db "CAMERICA OK", 0
txt_bad: .db "CAMERICA FAIL", 0

.org $FFFA
    .dw spin, start, spin
