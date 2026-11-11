; hello.asm - SheikhEmu self-made 16 KiB NROM test cartridge.
;
; 100% original content written for this project: no copyrighted data,
; no third-party code, no trademarked assets. Its purpose is to give M1
; a real cartridge to boot through the memory bus: it clears RAM, then
; increments two counters forever, publishing them where CPU-bus tests
; can observe them.
;
; Layout: NROM (mapper 0), 16 KiB PRG at $C000-$FFFF, 8 KiB CHR (blank).

    .org $C000

reset:
    sei                 ; no interrupts during setup
    cld                 ; binary arithmetic (no decimal mode on the 2A03)
    ldx #$00
clear_zero_page:
    lda #$00
    sta $00,x           ; clear zero page $00-$FF
    inx
    bne clear_zero_page

    lda #$00
    sta $10             ; tick counter
    sta $11             ; lap counter (256 ticks per lap)

main:
    inc $10
    bne no_lap
    inc $11
no_lap:
    lda $10
    sta $0200           ; publish the tick where tests can read it
    lda $11
    sta $0201           ; publish the lap
    jmp main

nmi:
    rti

irq:
    rti

; Interrupt vectors close the 16 KiB bank: NMI, RESET, IRQ.
    .org $FFFA
    .dw nmi, reset, irq
