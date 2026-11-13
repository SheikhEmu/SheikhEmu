; banktest_uxrom.asm — UxROM (iNES mapper 2) bank verification ROM.
;
; The engine lives in the fixed last 16 KiB bank at $C000. Each
; switchable bank starts with the signature $B0+i and carries $B4+i
; at its tail ($BFFF). The test writes each selector value to the
; UxROM register ($8000-$FFFF), then reads the window at $8000 and
; $BFFF and compares. Results use the blargg-style WRAM protocol
; ($6000 status, $6001/$6002 = $DE/$B0, $6004.. text).

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
    ldx #$00                ; test index 0..2
loop:
    lda sel,x
    sta $8000               ; UxROM bank register
    lda $8000               ; window start signature
    cmp prglo,x
    bne fail
    lda $BFFF               ; window tail signature
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
    inx                     ; 1-based failing test number
    stx WSTAT
    ldy #$00
wbad:
    lda txt_bad,y
    beq spin
    sta WTXT,y
    iny
    bne wbad

; Selectors and expected signatures (data banks 0..2 of four).
sel:     .db $00, $01, $02
prglo:   .db $B0, $B1, $B2
prghi:   .db $B4, $B5, $B6
txt_ok:  .db "UXROM OK", 0
txt_bad: .db "UXROM FAIL", 0

.org $FFFA
    .dw spin, start, spin
