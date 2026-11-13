; banktest_gxrom.asm — GxROM (iNES mapper 66) verification.
;
; One register switches BOTH the 32 KiB PRG bank (value bits 6-7) and
; the 8 KiB CHR bank (value bits 0-1). Same engine shape as
; ColorDreams: RAM stub, PRG bank held at 1 while the CHR banks sweep
; 0..3.

.org $8000
.equ WSTAT, $6000
.equ WDE,   $6001
.equ WBO,   $6002
.equ WTXT,  $6004
.equ STUB,  $0200
.equ CHRL,  $F0
.equ CHRH,  $F1

start:
    lda #$80                ; status: running
    sta WSTAT
    lda #$DE
    sta WDE
    lda #$B0
    sta WBO
    ; Copy the RAM stub (terminated by the $AA sentinel byte).
    ldx #$00
copy:
    lda stub,x
    cmp #$AA
    beq copy_done
    sta STUB,x
    inx
    bne copy
copy_done:
    ldx #$00                ; test index 0..3
loop:
    lda #$00                ; PPU address -> $0000 for the CHR probe
    sta $2006
    sta $2006
    lda sel,x
    jsr STUB                ; PRG signature returns in A
    cmp prglo,x
    bne fail
    lda CHRL
    cmp chrlo,x
    bne fail
    lda CHRH
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

; The RAM stub: A = register value on entry.
stub:
    sta $8000               ; PRG + CHR banks switch together
    lda $8000               ; PRG signature of the new bank
    pha
    lda $2007               ; discard the buffered read
    lda $2007               ; CHR at $0000
    sta CHRL
    lda #$1F
    sta $2006
    lda #$FF                ; PPU address -> $1FFF
    sta $2006
    lda $2007               ; discard
    lda $2007               ; CHR at $1FFF
    sta CHRH
    lda #$00
    sta $8000               ; back to bank 0 (the engine)
    pla
    rts
    .db $AA                 ; stub terminator (never executed)

; Selectors: PRG bank 1 (bits 6-7) + CHR bank 0..3 (bits 0-1).
sel:     .db $40, $41, $42, $43
prglo:   .db $B1, $B1, $B1, $B1
prghi:   .db $B5, $B5, $B5, $B5
chrlo:   .db $C0, $C1, $C2, $C3
chrhi:   .db $C4, $C5, $C6, $C7
txt_ok:  .db "GXROM OK", 0
txt_bad: .db "GXROM FAIL", 0

.org $FFFA
    .dw spin, start, spin
