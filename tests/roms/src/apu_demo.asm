; apu_demo.asm - SheikhEmu self-made APU demonstration cartridge (M4.4).
;
; 100% original content written for this project: no copyrighted data,
; no third-party code, no sampled audio, no trademarked assets. Every
; byte of the sound is synthesized by the five APU channels from first
; principles, so the whole ROM is homebrew built with the project's own
; toolchain (tools/asm/mini_asm.py + tools/mkrom.py via make test-roms).
; The silent brick game is not touched by this file in any way.
;
; Purpose: an owned sound-test ROM whose five channels play tones at
; PUBLISHED frequencies (NESdev Wiki formulas, NTSC CPU clock
; 1789772.727 Hz = 21477272.727/12), each channel isolated in its own
; timed window so the native-rate PCM stream can be measured digitally
; (the M4.6 measurement pass) window by window:
;
;   phase 1  pulse 1   t=$FD  f = CPU/(16*(t+1)) = 440.40 Hz   (A4)
;   phase 2  pulse 2   t=$7F  f = CPU/(16*(t+1)) = 873.91 Hz   (A5)
;   phase 3  triangle  t=$FD  f = CPU/(32*(t+1)) = 220.20 Hz   (A3,
;            exactly one octave below phase 1 - same timer value)
;   phase 4  noise     short mode (bit 7 of $400E), period index
;            4 = published NTSC table entry 64 CPU cycles; the
;            93-step short-mode sequence repeats every 93*64 = 5952
;            cycles -> 300.70 Hz pattern rate
;   phase 5  DMC       rate index 0 = published NTSC table entry
;            428 CPU cycles per output tick; the looped 33-byte
;            sample alternates $FF/$00 (16-tick level triangles),
;            fundamental = CPU/(16*428) = 261.36 Hz (C4)
;   phase 6  all five channels together (the ensemble)
;   phases 0 and 7    silence (the measurement baselines)
;
; Both pulses use duty 2 (a 50% square, the cleanest waveform for
; zero-crossing measurement) at constant volume 15; the triangle runs
; its full 32-step sequencer; the noise uses halt+constant volume 15.
;
; The demo is driven by the vblank NMI: one interrupt per frame, a
; frame counter in $11, a phase counter in $10, and one register-write
; script per phase walked by a small interpreter (the (zp),y + abs,X
; pair). The phase-script pointer table is copied into zero page
; $30-$3F at reset (the assembler has no label arithmetic, so the
; high bytes are read at base $31 with the same index). Each phase
; lasts 128 frames (~2.13 s at 60.0988 fps); the eight phases loop
; forever (the demo replays every 1024 frames, ~17.04 s). The register
; scripts write $4015 first for the pulse/triangle/noise phases (a
; fourth-register write only reloads the length counter while the
; channel is enabled - the published rule the M4.2 core implements)
; and $4015 last for the DMC phase (the sample address/length must be
; in place before the restart).
;
; Layout: NROM (mapper 0), 16 KiB PRG at $C000-$FFFF, 8 KiB blank
; CHR-ROM - the exact board of hello.nes. The DMC sample lives at
; $E000, the vectors close the bank.

    .org $C000

    .equ PHASE_LEN, $80

reset:
    sei                          ; no interrupts during setup
    cld                          ; binary arithmetic (no decimal mode)
    ldx #$00
    lda #$00
clear_zero_page:
    sta $00,x                    ; clear zero page $00-$FF
    inx
    bne clear_zero_page

    lda #$00
    sta $10                      ; phase counter 0..7
    sta $11                      ; frames elapsed inside the phase
    sta $4015                    ; explicit silence from the first cycle

    lda #$40
    sta $4017                    ; inhibit the frame-counter IRQ

    lda #$80
    sta $2000                    ; enable vblank NMI (rendering stays off)

    ; Copy the phase-script pointer table (16 bytes) into zero page
    ; $30-$3F: the NMI reads the low byte at $30+Y and the high byte at
    ; $31+Y with the same doubled phase index.
    ldx #$00
copy_table:
    lda phase_table,x            ; absolute,X reads the ROM words
    sta $30,x                    ; zero-page,X writes the copy
    inx
    cpx #$10
    bne copy_table

main:
    jmp main                     ; the NMI does all the work from here

; One vblank NMI per frame: count frames inside the phase, advance the
; phase every PHASE_LEN frames, and on the first NMI of each new phase
; run that phase's register script. A/X/Y are free to use: the main
; loop holds no register state.
nmi:
    inc $11
    lda $11
    cmp #PHASE_LEN
    bcc nmi_dispatch
    lda #$00
    sta $11
    inc $10
    lda $10
    and #$07                     ; the demo loops: phase 7 -> 0
    sta $10
nmi_dispatch:
    lda $11
    bne nmi_out                  ; act only on the phase's first NMI
    lda $10
    asl a                        ; phase * 2 = byte index into the table
    tay
    lda $30,y                    ; the script pointer low byte
    sta $20
    lda $31,y                    ; the script pointer high byte
    sta $21
    jsr run_script
nmi_out:
    rti

; Walk the (offset, value) pairs at ($20): each value is stored to
; $4000+offset; the offset $FF terminates the script.
run_script:
    ldy #$00
run_next:
    lda ($20),y                  ; the register offset (or $FF)
    cmp #$FF
    beq run_done
    tax                          ; X = offset $00..$17
    iny
    lda ($20),y                  ; the value
    sta $4000,x                  ; absolute,X reaches $4000-$4017
    iny
    jmp run_next
run_done:
    rts

irq:
    rti

; ---- the phase scripts --------------------------------------------------
; (offset, value) pairs; offset $FF terminates. Order inside a script
; matters: $4015 FIRST for pulse/triangle/noise (fourth-register writes
; reload the length only while enabled), $4015 LAST for the DMC.

script_0:                        ; phase 0: explicit silence
    .db $15,$00
    .db $FF

script_1:                        ; phase 1: pulse 1 solo, A4 440.40 Hz
    .db $15,$01                  ; pulse 1 on
    .db $00,$BF                  ; duty 2, halt, constant volume 15
    .db $01,$00                  ; sweep off (no muting)
    .db $02,$FD                  ; timer low  (t = $0FD)
    .db $03,$08                  ; length idx 1, timer high 0
    .db $FF

script_2:                        ; phase 2: pulse 2 solo, A5 873.91 Hz
    .db $15,$02                  ; pulse 2 on
    .db $04,$BF                  ; duty 2, halt, constant volume 15
    .db $05,$00                  ; sweep off
    .db $06,$7F                  ; timer low  (t = $07F)
    .db $07,$08                  ; length idx 1, timer high 0
    .db $FF

script_3:                        ; phase 3: triangle solo, A3 220.20 Hz
    .db $15,$04                  ; triangle on
    .db $08,$FF                  ; control on, linear reload 127
    .db $0A,$FD                  ; timer low  (t = $0FD)
    .db $0B,$08                  ; length idx 1, timer high 0
    .db $FF

script_4:                        ; phase 4: noise, short mode, 300.70 Hz
    .db $15,$08                  ; noise on
    .db $0C,$3F                  ; halt, constant volume 15
    .db $0E,$84                  ; short mode, period index 4 (entry 64)
    .db $0F,$08                  ; length idx 1
    .db $FF

script_5:                        ; phase 5: DMC loop, 261.36 Hz triangles
    .db $12,$80                  ; sample at $C000 + $80*64 = $E000
    .db $13,$02                  ; length ($02*16)+1 = 33 bytes
    .db $10,$40                  ; loop on, no IRQ, rate index 0 (428)
    .db $15,$10                  ; DMC on - starts the looped playback
    .db $FF

script_6:                        ; phase 6: all five channels together
    .db $15,$1F                  ; everything on
    .db $00,$BF                  ; pulse 1: A4
    .db $01,$00
    .db $02,$FD
    .db $03,$08
    .db $04,$BF                  ; pulse 2: A5
    .db $05,$00
    .db $06,$7F
    .db $07,$08
    .db $08,$FF                  ; triangle: A3
    .db $0A,$FD
    .db $0B,$08
    .db $0C,$3F                  ; noise: short mode 300.70 Hz
    .db $0E,$84
    .db $0F,$08
    .db $12,$80                  ; DMC: same looped sample
    .db $13,$02
    .db $10,$40
    .db $FF

script_7:                        ; phase 7: silence again
    .db $15,$00
    .db $FF

phase_table:
    .dw script_0, script_1, script_2, script_3
    .dw script_4, script_5, script_6, script_7

; ---- the DMC sample (33 bytes at $E000) ---------------------------------
; Alternating $FF/$00 bytes give 16-tick level triangles (0..16); the
; final $0F (four one-bits then four zero-bits, consumed LSB first)
; balances the loop at exactly 132 one-bits and 132 zero-bits, so the
; level returns to its base every loop - no drift, no clamping
; transients, a purely periodic stream.
    .org $E000
dmc_sample:
    .db $FF,$00,$FF,$00,$FF,$00,$FF,$00
    .db $FF,$00,$FF,$00,$FF,$00,$FF,$00
    .db $FF,$00,$FF,$00,$FF,$00,$FF,$00
    .db $FF,$00,$FF,$00,$FF,$00,$FF,$00
    .db $0F

; Interrupt vectors close the 16 KiB bank: NMI, RESET, IRQ.
    .org $FFFA
    .dw nmi, reset, irq
