; banktest_axrom.asm — AxROM (iNES mapper 7) bank verification.
;
; The engine is bank 0 (the power-on 32 KiB bank). Because the whole
; $8000-$FFFF window switches, the test copies a small stub into RAM
; ($0200) that performs the switch, reads the signature, and restores
; bank 0 before returning — the classic technique real games use on
; whole-window boards. CHR is 8 KiB RAM on this board, so the CHR
; probes (kept to share the engine shape with ColorDreams/GxROM)
; expect zeros. The single-screen page bit is exercised in the Rust
; unit tests (it needs no cartridge-side proof).

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
    ldx #$00                ; test index 0..2
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

; The RAM stub: A = register value on entry. Switches the window,
; samples the PRG signature and the two CHR probes, restores bank 0,
; and returns the PRG signature in A.
stub:
    sta $8000               ; switch the whole $8000-$FFFF window
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

; Selectors and expected signatures (data banks 1..3 of four).
sel:     .db $01, $02, $03
prglo:   .db $B1, $B2, $B3
prghi:   .db $B5, $B6, $B7
chrlo:   .db $00, $00, $00  ; CHR RAM: zeros
chrhi:   .db $00, $00, $00
txt_ok:  .db "AXROM OK", 0
txt_bad: .db "AXROM FAIL", 0

.org $FFFA
    .dw spin, start, spin
