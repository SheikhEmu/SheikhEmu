; banktest_j87.asm — J87 (iNES mapper 87) CHR bank verification.
;
; PRG is fixed; the CHR bank register listens on writes to
; $6000-$7FFF with the two bank bits REVERSED (bit 0 = high bit,
; bit 1 = low bit), so the selector sequence for banks 0..3 is
; $00,$02,$01,$03. Note the protocol writes themselves land in
; $6000-$6FFF and therefore touch the register — the sweep runs
; afterwards and overwrites the selection each time, so the report
; writes never disturb a pending verification.

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
    sta $7000               ; J87 CHR bank register ($6000-$7FFF)
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

; Reversed-bit selectors for banks 0..3, and expected CHR signatures.
sel:     .db $00, $02, $01, $03
chrlo:   .db $C0, $C1, $C2, $C3
chrhi:   .db $C4, $C5, $C6, $C7
txt_ok:  .db "J87 OK", 0
txt_bad: .db "J87 FAIL", 0

.org $FFFA
    .dw spin, start, spin
