; brick.asm — SHEIKHEMU BRICK: the M3.4 playable game (UxROM, mapper 2).
;
; 100% original content written for this project: no copyrighted data,
; no third-party code, no trademarked assets. A "tower demolition"
; breakout: a paddle, a ball, and brick towers that grow taller and
; faster each level.
;
; Board (UxROM, 64 KiB PRG = 4 x 16 KiB):
;   bank 3 (fixed $C000-$FFFF) : this engine;
;   banks 0..2 (window $8000-$BFFF): one per level — the layout, the
;   palette and the speed parameters. Level advance REALLY writes the
;   bank register ($8000) and the engine reads the new level's data
;   through the switched window (mkbrick.py builds the banks).
;
; Playfield (pixels): walls at x 0..15 / 240..255, ceiling y 16..23,
; bricks from y 32 (6 rows of 8 px, 14 cells of 16 px), paddle y 208
; (40 px wide), ball lost below y 224. The serve re-aligns the paddle
; at the field center (documented design choice: every serve is fair).
;
; WRAM protocol ($6000, blargg-style; PRG-RAM present):
;   $6000 status ($80 running, $00 you-win, $0E game-over),
;   $6001/$6002 = $DE/$B0, $6004.. state text,
;   $6010 level, $6011 lives, $6012 outcome (0/1 over/2 win),
;   $6013 bank writes, $6014 bricks destroyed, $6015 current bank,
;   $6016/$6017 score (16-bit binary).

; ---- zero page ------------------------------------------------------------
.equ STATE,   $00      ; 0 attract 1 serve 2 play 3 over 4 win
.equ DELAY,   $01     ; sub-state frame counter
.equ LEVEL,   $02     ; 0..2 (== bank id)
.equ LIVES,   $03     ; 3..0
.equ PADX,    $04     ; paddle left edge (16..200)
.equ BALLX,   $05     ; ball left edge (16..232)
.equ BALLY,   $06     ; ball top edge (24..224)
.equ DX,      $07     ; ball velocity x ($FF/0/$01)
.equ DY,      $08     ; ball velocity y ($FF/$01)
.equ SPEED,   $09     ; ball 1-px sub-steps per frame (2 or 3)
.equ PTS,     $0A     ; points per brick (this level)
.equ BRICKS,  $0B     ; bricks left in this level
.equ NMIFLAG, $0C     ; the NMI handshake
.equ INPUT,   $0D     ; button bitmap (bit0=A .. bit7=Right)
.equ PREVIN,  $0E     ; previous bitmap (edge detection)
.equ SCOREL,  $10     ; score, 16-bit binary little endian
.equ SCOREH,  $11
.equ QLEN,    $14     ; pending VRAM queue entries
.equ CURBANK, $15     ; bank value last written to $8000
.equ BANKWR,  $16     ; bank writes so far (the switching evidence)
.equ DESTROY, $17     ; bricks destroyed (running total)
.equ TMPA,    $18     ; queue address (hi)
.equ TMPB,    $19     ; queue address (lo)
.equ QOFF,    $1A     ; queue byte-offset scratch
.equ DIGU,    $1B     ; score digits: units, tens, hundreds, thou
.equ DIGT,    $1C
.equ DIGH,    $1D
.equ DIGTH,   $1E
.equ SRCP,    $1C     ; NOTE: SRCP/SRCPH share DIGT/DIGH deliberately:
.equ SRCPH,   $1D     ; strings and digits never run concurrently.
.equ SCR1,    $21     ; scratch: row
.equ SCR2,    $22     ; scratch: col
.equ SCR3,    $23     ; scratch: value / index
.equ SCR4,    $24     ; scratch: Y save inside char_tile
.equ SCRW1,   $25     ; 16-bit score scratch (lo/hi)
.equ SCRW2,   $26
.equ BTILE,   $27     ; the level's brick tile
.equ CLEANPH, $28     ; field cleanup phase (3,2,1,0 = done)
.equ ZPL,     $2B     ; RAM clear pointer (lo/hi)
.equ ZPH,     $2C
.equ QHEAD,   $2D     ; drain position inside the pending queue (D-31)

; ---- fixed addresses ------------------------------------------------------
.equ PPUCTRL, $2000
.equ PPUMASK, $2001
.equ PPUSTAT, $2002
.equ PPUADDR, $2006
.equ PPUDATA, $2007
.equ OAMDMA,  $4014
.equ JOY1,    $4016

.equ QUEUE0,  $0300   ; VRAM write queue: (hi, lo, value) x <= 84
.equ QUEUE1,  $0301
.equ QUEUE2,  $0302
.equ QMAX,    84
.equ ALIVE,   $0500   ; brick grid: 6 rows x 14 cells
.equ OAM,     $0200   ; OAM shadow

.equ WSTAT,   $6000   ; WRAM protocol
.equ WDE,     $6001
.equ WBO,     $6002
.equ WTXT,    $6004
.equ TLVL,    $6010
.equ TLIV,    $6011
.equ TOUT,    $6012
.equ TBWR,    $6013
.equ TBRK,    $6014
.equ TCBK,    $6015
.equ TSCR,    $6016
.equ TSCRH,   $6017

; button bits as the read loop accumulates them (rol: the FIRST
; serial read — A — lands in bit 7, the last — Right — in bit 0).
.equ BTN_A,     $80
.equ BTN_B,     $40
.equ BTN_SELECT, $20
.equ BTN_START, $10
.equ BTN_UP,    $08
.equ BTN_DOWN,  $04
.equ BTN_LEFT,  $02
.equ BTN_RIGHT, $01

.equ PAD_CENTER, 116
.equ BALL_TOP,   200
.equ PADDLE_Y,   208
.equ LOST_Y,     224
.equ BRICK_TOP,  32
.equ FIELD_L,    16
.equ FIELD_R,    232

.org $C000

; ---------------------------------------------------------------------------
reset:
    sei
    cld
    ldx #$FF
    txs

    ; clear the whole 2 KiB RAM (the shadow pages live inside it).
    lda #$00
    sta ZPL
    lda #$00
    sta ZPH
    ldx #$08
clr_page:
    ldy #$00
    lda #$00
clr_byte:
    sta (ZPL),y
    iny
    bne clr_byte
    inc ZPH
    dex
    bne clr_page

    ; WRAM protocol + telemetry init.
    lda #$80
    sta WSTAT
    lda #$DE
    sta WDE
    lda #$B0
    sta WBO
    lda #$00
    sta TOUT
    sta TBWR
    sta TBRK
    sta TCBK
    sta TSCR
    sta TSCRH
    sta CURBANK
    ldy #$00
wtxt_boot:
    lda txt_attract,y
    beq wtxt_boot_done
    sta WTXT,y
    iny
    bne wtxt_boot
wtxt_boot_done:

    ; two vblanks: the documented PPU warm-up.
wait1:
    bit PPUSTAT
    bpl wait1
wait2:
    bit PPUSTAT
    bpl wait2

    ; ---- CHR RAM: tiles 0..7 at $0000, digits at $0100, letters $0200 --
    ; D-31: every tile is a full 16-byte two-plane tile (the PPU
    ; always decodes 16 bytes per tile; the old 8-byte "tiles" made
    ; each glyph's plane 1 the next glyph's data -> on-screen garble).
    bit PPUSTAT
    lda #$00
    sta PPUADDR
    lda #$00
    sta PPUADDR
    ldx #$0C
    jsr load_src            ; slot offset 12: chr_tiles
    lda #$80
    sta SCR3
    jsr chr_bytes           ; 128 bytes = 8 tiles x 16

    bit PPUSTAT
    lda #$01
    sta PPUADDR
    lda #$00
    sta PPUADDR
    ldx #$0E
    jsr load_src            ; slot offset 14: font_digits
    lda #$A0
    sta SCR3
    jsr chr_bytes           ; 160 bytes = 10 digits x 16

    bit PPUSTAT
    lda #$02
    sta PPUADDR
    lda #$00
    sta PPUADDR
    ldx #$10
    jsr load_src            ; slot offset 16: font_letters
    lda #$00
    sta SCR3
    jsr chr_bytes           ; 256 bytes = letters 0..15 (D-31 two-plane)
    inc SRCPH               ; the source pointer advances 256
    lda #$40
    sta SCR3
    jsr chr_bytes           ; 64 more = letters 16..19, fills to $033F

    ; ---- the base nametable: ceiling row 2, walls in every row -------
    bit PPUSTAT
    lda #$20
    sta PPUADDR
    lda #$40
    sta PPUADDR
    ldx #$20
ceil_loop:
    lda #$01
    sta PPUDATA
    dex
    bne ceil_loop

    lda #$03
    sta SCR1                ; tile rows 3..29
row_loop:
    lda SCR1
    sta SCR3
    lda #$00
    sta SCR4
    ldx #$05
row_shl:
    asl SCR3
    rol SCR4
    dex
    bne row_shl
    lda #$20
    clc
    adc SCR4
    sta PPUADDR
    lda SCR3
    sta PPUADDR
    ldy #$00
row_cols:
    lda row_pat,y
    sta PPUDATA
    iny
    cpy #$20
    bne row_cols
    inc SCR1
    lda SCR1
    cmp #$1E
    bne row_loop

    ; attributes: the brick band (tile rows 4..9, attribute bytes
    ; 8..$17 covering tile rows 4..11) uses bg palette 1 for the
    ; inner quadrants; the side-wall quadrants (cols 0..1 / 30..31)
    ; stay palette 0 so the walls keep one gray ink top to bottom
    ; (no red side-wall segments inside the brick band).
    bit PPUSTAT
    lda #$23
    sta PPUADDR
    lda #$C0
    sta PPUADDR
    ldx #$00
attr_loop:
    lda #$00
    cpx #$08
    bcc attr_put
    cpx #$18
    bcs attr_put
    txa
    and #$07
    beq attr_left            ; col-group 0: walls left, bricks right
    cmp #$07
    beq attr_right           ; col-group 7: bricks left, walls right
    lda #$55                 ; middle col-groups: all palette 1
    jmp attr_put
attr_left:
    lda #$44                 ; wall quadrants palette 0, brick quadrants palette 1
    jmp attr_put
attr_right:
    lda #$11                 ; brick quadrants palette 1, wall quadrants palette 0
attr_put:
    sta PPUDATA
    inx
    cpx #$40
    bne attr_loop

    ; ---- the attract palette -------------------------------------------
    ; D-31: bg palette 0 color 1 is the walls/ceiling ink ($10 gray)
    ; and color 3 the two-plane text ink ($30 white); the old $00
    ; entries were black-on-black invisible.
    bit PPUSTAT
    lda #$3F
    sta PPUADDR
    lda #$00
    sta PPUADDR
    ldx #$00
pal_boot:
    lda pal_attract,x
    sta PPUDATA
    inx
    cpx #$20
    bne pal_boot

    ; ---- the attract text and its tower -------------------------------
    bit PPUSTAT
    lda #$20
    sta PPUADDR
    lda #$CC                 ; row 6, col 12: SHEIKHEMU
    sta PPUADDR
    ldx #$00
    jsr load_src            ; slot 0: txt_title
    jsr ppu_str

    bit PPUSTAT
    lda #$21
    sta PPUADDR
    lda #$0C                 ; row 8, col 12: BREAKOUT
    sta PPUADDR
    ldx #$02
    jsr load_src            ; slot 1: txt_sub
    jsr ppu_str

    bit PPUSTAT
    lda #$22
    sta PPUADDR
    lda #$2A                 ; row 17, col 10: PRESS START
    sta PPUADDR
    ldx #$04
    jsr load_src            ; slot 2: txt_press
    jsr ppu_str

    ; the attract tower: three stacked brick pairs at rows 12..14,
    ; columns 16..17 - the exact cell the level towers grow from
    ; (TOWER_COL 7 -> tile columns 16..17), so the title previews the
    ; play itself (IMPROVE: clean transition, one shape language).
    bit PPUSTAT
    lda #$21
    sta PPUADDR
    lda #$90                 ; row 12, col 16
    sta PPUADDR
    lda #$02
    sta PPUDATA
    lda #$02
    sta PPUDATA
    bit PPUSTAT
    lda #$21
    sta PPUADDR
    lda #$B0                 ; row 13, col 16
    sta PPUADDR
    lda #$02
    sta PPUDATA
    lda #$02
    sta PPUDATA
    bit PPUSTAT
    lda #$21
    sta PPUADDR
    lda #$D0                 ; row 14, col 16
    sta PPUADDR
    lda #$02
    sta PPUDATA
    lda #$02
    sta PPUDATA

    ; ---- the OAM shadow -------------------------------------------------
    ldx #$00
oam_init:
    txa
    and #$03
    bne oam_zero
    lda #$F0
    jmp oam_put
oam_zero:
    lda #$00
oam_put:
    sta OAM,x
    inx
    bne oam_init

    ; rendering on: background + sprites, NMI on.
    ; D-31: the canonical scroll reset after the boot VRAM writes -
    ; $2000 (nametable latch into t) then two $2005 writes (0,0), so
    ; the first frame renders from nametable 0 at scroll 0,0 instead
    ; of wherever the last $2006 left the loopy t register.
    lda #$80
    sta PPUCTRL
    lda #$00
    sta $2005
    sta $2005
    lda #$1E
    sta PPUMASK

    lda #$00                 ; the attract state
    sta STATE
    sta DELAY

; ---------------------------------------------------------------------------
main_loop:
    lda NMIFLAG
    beq main_loop
    lda #$00
    sta NMIFLAG
    jsr read_input
    lda STATE
    cmp #$01
    beq do_serve
    cmp #$02
    beq do_play
    cmp #$03
    beq do_over
    cmp #$04
    beq do_win
    jmp do_attract

do_attract:
    lda INPUT
    and #BTN_START
    bne go_init
    inc DELAY
    beq go_init             ; 256 frames: auto-advance
    jmp main_done
go_init:
    jmp init_game

do_serve:
    jsr serve_step
    jmp main_done

do_play:
    jsr play_step
    jmp main_done

do_over:
    lda INPUT
    and #BTN_START
    bne go_init2
    jmp main_done
go_init2:
    jmp init_game

do_win:
    lda INPUT
    and #BTN_START
    beq win_wait
    jmp init_game
win_wait:

main_done:
    lda STATE
    beq md_no_oam           ; attract: the game sprites stay hidden
    jsr shadow_oam
md_no_oam:
    jsr wram_update
    jmp main_loop

; ---------------------------------------------------------------------------
init_game:
    lda #$00
    sta SCOREL
    sta SCOREH
    sta DESTROY
    sta LEVEL
    lda #$03
    sta LIVES
    lda #$80
    sta WSTAT
    lda #$00
    sta TOUT
    lda #$04                 ; the cleanup phase counter (4 batches)
    sta CLEANPH
    lda #$01
    sta STATE
    sta DELAY
    jmp main_loop

; The serve phase: the field cleanup (attract/over/win leftovers)
; and the level load, one bounded batch per frame and only once the
; previous batch has fully drained (the queue is drained at most 32
; entries per NMI - see nmi); then the actual serve delay.
serve_step:
    lda QLEN
    bne serve_draining     ; last batch still in flight
    lda CLEANPH
    beq serve_ready
    cmp #$04
    bne sc3_check2
    lda #$06                 ; row 6: title
    jsr clean_row
    lda #$08                 ; row 8: subtitle
    jsr clean_row
    dec CLEANPH
    rts
sc3_check2:
    cmp #$03
    bne sc2
    lda #$0C                 ; row 12: over-text / attract tower
    jsr clean_row
    lda #$0D
    jsr clean_row
    lda #$0E
    jsr clean_row
    dec CLEANPH
    rts
sc2:
    cmp #$02
    bne sc1a
    lda #$11                 ; row 17: the attract PRESS START
    jsr clean_row
    lda #$14                 ; row 20: the over/win PRESS START
    jsr clean_row
    dec CLEANPH
    rts
sc1a:
    cmp #$01
    bne sc1b
    dec CLEANPH             ; fall through: this frame loads the level
sc1b:
    lda #$00
    sta CLEANPH
    jsr load_level
    jsr status_level
    jsr wram_level_text
    rts
serve_draining:
    rts

serve_ready:
    inc DELAY
    lda DELAY
    cmp #$5A                 ; 90 frames auto-launch
    bcs serve_launch
    lda INPUT
    and #BTN_A
    bne serve_launch
    jsr recenter_ball
    rts
serve_launch:
    lda #$FF
    sta DY                   ; up
    lda #$00
    sta DX                   ; the centered serve: straight
    lda #$02
    sta STATE
    rts

recenter_ball:
    lda #PAD_CENTER
    sta PADX
    lda PADX
    clc
    adc #$10
    sta BALLX
    lda #BALL_TOP
    sta BALLY
    rts

; ---------------------------------------------------------------------------
play_step:
    lda INPUT
    and #BTN_LEFT
    beq try_right
    lda PADX
    sec
    sbc #$02
    bcs pad_store_l
    lda #$10
pad_store_l:
    sta PADX
try_right:
    lda INPUT
    and #BTN_RIGHT
    beq pad_done
    lda PADX
    clc
    adc #$02
    cmp #$C9
    bcc pad_store_r
    lda #$C8
pad_store_r:
    sta PADX
pad_done:
    ldx SPEED
ball_loop:
    jsr ball_step
    lda STATE
    cmp #$02
    bne ball_out
    dex
    bne ball_loop
ball_out:
    rts

; one 1-px ball sub-step.
ball_step:
    lda BALLX
    clc
    adc DX
    sta BALLX
    lda BALLY
    clc
    adc DY
    sta BALLY

    lda BALLX
    cmp #$10
    bcs chk_right
    lda #$10
    sta BALLX
    lda #$01
    sta DX
chk_right:
    lda BALLX
    cmp #FIELD_R
    bcc chk_ceil
    lda #FIELD_R
    sta BALLX
    lda #$FF
    sta DX
chk_ceil:
    lda BALLY
    cmp #$18
    bcs chk_paddle
    lda #$18
    sta BALLY
    lda #$01
    sta DY

chk_paddle:
    lda DY
    bmi chk_loss
    lda BALLY
    clc
    adc #$08
    cmp #PADDLE_Y
    bcc chk_loss             ; still above the paddle line
    lda BALLY
    cmp #$E8
    bcs life_lost
    jsr hit_paddle
    rts
chk_loss:
    lda BALLY
    cmp #LOST_Y
    bcs life_lost
    jsr hit_bricks
    rts

life_lost:
    dec LIVES
    lda LIVES
    beq game_over
    lda #$01
    sta STATE
    sta DELAY
    rts

game_over:
    lda #$03
    sta STATE
    lda #$0E
    sta WSTAT
    lda #$01
    sta TOUT
    ldx #$06
    jsr load_src             ; slot offset 6: txt_over
    jsr wram_text
    ; GAME OVER at row 12, col 11: $218B.
    lda #$21
    sta TMPA
    lda #$8B
    sta TMPB
    jsr queue_str
    ldx #$04
    jsr load_src             ; slot offset 4: txt_press
    lda #$22
    sta TMPA
    lda #$8A
    sta TMPB
    jsr queue_str
    rts

you_win:
    lda #$04
    sta STATE
    lda #$00
    sta WSTAT
    lda #$02
    sta TOUT
    ldx #$08
    jsr load_src             ; slot offset 8: txt_win
    jsr wram_text
    lda #$21
    sta TMPA
    lda #$8C
    sta TMPB
    jsr queue_str
    ldx #$04
    jsr load_src             ; slot 2: txt_press
    lda #$22
    sta TMPA
    lda #$8A
    sta TMPB
    jsr queue_str
    rts

hit_paddle:
    lda BALLX
    clc
    adc #$08
    cmp PADX
    bcc paddle_ret           ; ball right < paddle left
    lda PADX
    clc
    adc #$28
    cmp BALLX
    bcc paddle_ret           ; paddle right < ball left
    lda #$FF
    sta DY
    lda BALLX
    clc
    adc #$04
    sec
    sbc PADX                 ; hit offset 0..47
    cmp #$0C
    bcc pad_dx_neg
    cmp #$1C
    bcc pad_dx_zero
    lda #$01
    jmp pad_dx_store
pad_dx_neg:
    lda #$FF
    jmp pad_dx_store
pad_dx_zero:
    lda #$00
pad_dx_store:
    sta DX
paddle_ret:
    rts

; the brick collision: destroy + reflect + score + level end.
hit_bricks:
    lda BALLY
    clc
    adc #$04
    sec
    sbc #BRICK_TOP
    bcc hb_out               ; above the brick band
    cmp #$30
    bcs hb_out               ; below the brick band
    lsr a
    lsr a
    lsr a                    ; row 0..5
    sta SCR1
    lda BALLX
    clc
    adc #$04
    sec
    sbc #FIELD_L
    bcc hb_out               ; left of the field
    lsr a
    lsr a
    lsr a
    lsr a                    ; col 0..13
    cmp #$0E
    bcs hb_out
    sta SCR2
    ; index = row*14 + col
    lda SCR1
    asl a
    asl a
    asl a
    asl a                    ; row*16
    sec
    sbc SCR1                 ; row*15
    sec
    sbc SCR1                 ; row*14
    clc
    adc SCR2
    tay
    lda ALIVE,y
    beq hb_out
    lda #$00
    sta ALIVE,y
    dec BRICKS
    inc DESTROY
    jsr cell_addr            ; SCR1 row / SCR2 col -> TMPA/TMPB
    lda #$00
    jsr queue_put
    inc TMPB
    bne hb_nc1
    inc TMPA
hb_nc1:
    lda #$00
    jsr queue_put
    lda PTS
    jsr add_score
    lda DY
    eor #$FE                 ; -1 <-> +1
    sta DY
    lda BRICKS
    bne hb_out
    jmp level_done
hb_out:
    rts

level_done:
    lda LEVEL
    cmp #$02
    bcs level_to_win
    inc LEVEL
    jsr load_level
    lda #$01
    sta STATE
    sta DELAY
    jsr wram_level_text
    rts
level_to_win:
    jsr you_win
    rts

; load the level in LEVEL through the switched bank window.
load_level:
    lda LEVEL
    sta $8000                ; the UxROM bank register
    sta CURBANK
    inc BANKWR
    ; clear the previous level's brick tiles first.
    ldx #$00
ll_clear:
    lda ALIVE,x
    beq ll_clear_next
    lda #$00
    sta ALIVE,x
    jsr queue_cell_tiles     ; BTILE = 0 here
ll_clear_next:
    inx
    cpx #$54
    bne ll_clear
    ; copy this level's layout from the window.
    ldx #$00
ll_copy:
    lda $8000,x
    sta ALIVE,x
    inx
    cpx #$54
    bne ll_copy
    ; count the bricks.
    ldx #$00
    lda #$00
    clc
ll_count:
    adc ALIVE,x
    inx
    cpx #$54
    bne ll_count
    sta BRICKS
    ; parameters at $8074: speed, points, brick tile.
    lda $8074
    sta SPEED
    lda $8075
    sta PTS
    lda $8076
    sta BTILE
    ; queue the level palette (32 bytes from $8054).
    ldx #$00
ll_pal:
    lda $8054,x
    sta SCR3
    lda #$3F
    sta TMPA
    stx TMPB
    lda SCR3
    jsr queue_put
    inx
    cpx #$20
    bne ll_pal
    ; draw this level's bricks (BTILE now holds the level tile).
    lda #$00
    sta BTILE
    ldx #$00
ll_draw:
    lda ALIVE,x
    beq ll_draw_next
    jsr queue_cell_tiles
ll_draw_next:
    inx
    cpx #$54
    bne ll_draw
    jsr recenter_ball
    rts

; queue the two tiles of brick cell X, using BTILE.
; The caller's X survives: the cell index rides the stack (the
; callees below reuse every scratch register).
queue_cell_tiles:
    txa
    pha
    sta SCR1                 ; the dividend (cell index)
    lda #$00
    sta SCR2                 ; col = 0
qct_div:
    lda SCR1
    cmp #$0E
    bcc qct_have
    sec
    sbc #$0E
    sta SCR1
    inc SCR2
    jmp qct_div
qct_have:
    jsr cell_addr            ; SCR1 row / SCR2 col -> TMPA/TMPB
    lda BTILE
    jsr queue_put
    inc TMPB
    bne qct_nc
    inc TMPA
qct_nc:
    lda BTILE
    jsr queue_put
    pla
    tax                      ; the caller's cell index restored
    rts

; the nametable address of brick cell (SCR1 row, SCR2 col).
cell_addr:
    lda SCR2
    asl a
    sta SCR2                 ; col*2
    lda SCR1
    clc
    adc #$04                 ; tile row 4..9
    sta SCR3
    lda #$00
    sta SCR4                 ; offset hi
    ldx #$05
ca_shl:
    asl SCR3
    rol SCR4                 ; (tile row)*32 as 16-bit
    dex
    bne ca_shl
    lda SCR2
    clc
    adc SCR3
    sta SCR3
    bcc ca_nc1
    inc SCR4
ca_nc1:
    lda #$02
    clc
    adc SCR3
    sta SCR3
    bcc ca_nc2
    inc SCR4
ca_nc2:
    lda SCR4
    clc
    adc #$20
    sta TMPA                 ; address hi
    lda SCR3
    sta TMPB                 ; address lo
    rts

; blank one inner field row (28 tiles, cols 2..29): A = tile row.
; Walls at cols 0-1 / 30-31 and rows outside 5..23 are untouched.
clean_row:
    sta SCR1
    lda #$00
    sta SCR2                 ; offset hi
    lda SCR1
    sta SCR3
    ldx #$05
crr_shl:
    asl SCR3
    rol SCR2                 ; row*32 as 16-bit
    dex
    bne crr_shl
    lda SCR2
    clc
    adc #$20
    sta TMPA                 ; address hi
    lda SCR3
    clc
    adc #$02                 ; skip the wall columns
    sta TMPB
    bcc crb_nc
    inc TMPA
crb_nc:
    ldy #$1C                 ; 28 inner columns
crb_loop:
    lda #$00
    jsr queue_put
    inc TMPB
    bne crb_nc2
    inc TMPA
crb_nc2:
    dey
    bne crb_loop
    rts

; ---------------------------------------------------------------------------
queue_put:
    ; A = value, TMPA/TMPB = full VRAM address.
    pha
    lda QLEN
    cmp #QMAX
    bcs qp_drop
    sta QOFF
    asl a
    clc
    adc QOFF                 ; 3*QLEN
    tay
    lda TMPA
    sta QUEUE0,y
    lda TMPB
    sta QUEUE1,y
    pla
    sta QUEUE2,y
    inc QLEN
    rts
qp_drop:
    pla
    rts

add_score:
    clc
    adc SCOREL
    sta SCOREL
    lda #$00
    adc SCOREH
    sta SCOREH
    jsr digits_from_score
    jsr status_score
    rts

digits_from_score:
    lda SCOREL
    sta SCRW1
    lda SCOREH
    sta SCRW2
    lda #$00
    sta DIGU
    sta DIGT
    sta DIGH
    sta DIGTH
df_loop:
    lda SCRW2
    bne df_ge10
    lda SCRW1
    cmp #$0A
    bcc df_done
df_ge10:
    lda SCRW1
    sec
    sbc #$0A
    sta SCRW1
    lda SCRW2
    sbc #$00
    sta SCRW2
    inc DIGT
    lda DIGT
    cmp #$0A
    bne df_loop
    lda #$00
    sta DIGT
    inc DIGH
    lda DIGH
    cmp #$0A
    bne df_loop
    lda #$00
    sta DIGH
    inc DIGTH
    jmp df_loop
df_done:
    lda SCRW1                ; remainder = units
    sta DIGU
    rts

status_score:
    ldx #$00
ss_loop:
    lda DIGU,x
    clc
    adc #$10
    sta SCR3
    lda #$20
    sta TMPA
    txa
    clc
    adc #$01                 ; cols 1..4
    sta TMPB
    lda SCR3
    jsr queue_put
    inx
    cpx #$04
    bne ss_loop
    rts

status_level:
    lda #$28                 ; the 'L' tile
    sta SCR3
    lda #$20
    sta TMPA
    lda #$08
    sta TMPB
    lda SCR3
    jsr queue_put
    lda LEVEL
    clc
    adc #$31                 ; ASCII '1'+level
    jsr char_tile
    sta SCR3
    lda #$20
    sta TMPA
    lda #$09
    sta TMPB
    lda SCR3
    jsr queue_put
    jsr status_lives
    rts

status_lives:
    ldx #$00
sl_loop:
    cpx LIVES
    bcc sl_show
    lda #$00
    jmp sl_put
sl_show:
    lda #$07                  ; the life icon tile
sl_put:
    sta SCR3
    lda #$20
    sta TMPA
    txa
    clc
    adc #$1A                  ; cols 26..28
    sta TMPB
    lda SCR3
    jsr queue_put
    inx
    cpx #$03
    bne sl_loop
    rts

; ---------------------------------------------------------------------------
wram_text:
    ldy #$00
wt_loop:
    lda (SRCP),y
    beq wt_done
    sta WTXT,y
    iny
    bne wt_loop
wt_done:
    lda #$00
    sta WTXT,y
    rts

wram_level_text:
    ldy #$00
wlt_loop:
    lda txt_level,y
    sta WTXT,y
    iny
    cpy #$06
    bne wlt_loop
    lda LEVEL
    clc
    adc #$31                 ; '1' + level
    sta WTXT,y
    iny
    lda #$00
    sta WTXT,y
    rts

ppu_str:
    ldy #$00
ps_loop:
    lda (SRCP),y
    beq ps_done
    jsr char_tile
    sta PPUDATA
    iny
    bne ps_loop
ps_done:
    rts

queue_str:
    ldy #$00
qs_loop:
    lda (SRCP),y
    beq qs_done
    sty SCR4                 ; queue_put clobbers Y: save the index
    jsr char_tile
    sta SCR3
    lda SCR3
    jsr queue_put
    inc TMPB
    bne qs_nc
    inc TMPA
qs_nc:
    ldy SCR4
    iny
    jmp qs_loop
qs_done:
    rts

char_tile:
    cmp #$20
    bne ct_digit
    lda #$00
    rts
ct_digit:
    cmp #$30
    bcc ct_letter
    cmp #$3A
    bcc ct_num
    jmp ct_letter
ct_num:
    sec
    sbc #$30
    clc
    adc #$10
    rts
ct_letter:
    cmp #$41
    bcc ct_blank
    cmp #$5B
    bcc ct_ok
ct_blank:
    lda #$00
    rts
ct_ok:
    sec
    sbc #$41
    sty SCR4
    tay
    lda letter_map,y
    ldy SCR4
    rts

; load the pointer slot X (two bytes) into SRCP/SRCPH.
load_src:
    lda txt_ptrs,x
    sta SRCP
    inx
    lda txt_ptrs,x
    sta SRCPH
    rts

; ---------------------------------------------------------------------------
read_input:
    lda #$01
    sta JOY1
    lda #$00
    sta JOY1
    lda #$00
    sta INPUT
    ldx #$08
ri_loop:
    lda JOY1
    lsr a                     ; bit 0 (pressed) -> carry
    rol INPUT
    dex
    bne ri_loop
    rts

; ---------------------------------------------------------------------------
; the OAM shadow: five paddle sprites, the ball (lives = HUD icons).
shadow_oam:
    ldx #$00                 ; X = OAM byte offset (sprite i = X/4)
so_pad:
    lda #PADDLE_Y
    sta $0200,x
    lda #$05
    sta $0201,x
    lda #$00
    sta $0202,x
    txa
    asl a                    ; X*2 = i*8: the paddle segment offset
    sta SCR3
    lda PADX
    clc
    adc SCR3
    sta $0203,x
    inx
    inx
    inx
    inx
    cpx #$14                 ; five sprites = 20 bytes
    bne so_pad
    lda BALLY
    sta $0214
    lda #$06
    sta $0215
    lda #$01
    sta $0216
    lda BALLX
    sta $0217
    ; The lives are the row-0 HUD icons (IMPROVE: one place, one
    ; language); the old duplicate sprite icons under the field are
    ; gone - their OAM bytes stay at the boot $F0 (offscreen).
    rts

; ---------------------------------------------------------------------------
chr_bytes:
    ; SRCP/SRCPH = source, SCR3 = count, PPUADDR preset.
    ldy #$00
cb_loop:
    lda (SRCP),y
    sta PPUDATA
    iny
    cpy SCR3
    bne cb_loop
    rts

; ---------------------------------------------------------------------------
nmi:
    pha
    txa
    pha
    tya
    pha

    lda #$00
    sta $2003
    lda #$02
    sta OAMDMA

    ; D-31: the CAPPED drain. The OAM DMA alone costs 513 bus cycles
    ; and each queue entry ~40, so a full 84-entry drain plus the DMA
    ; would run ~3300 cycles - past the ~2273-cycle vblank, into the
    ; pre-render line and past its v-reload, and every following
    ; frame then rendered from whatever $2006 the drain tail left in
    ; v (the frame-by-frame drift of the owner's videos). The NMI
    ; now drains at most 32 entries (about 1900 cycles with the DMA)
    ; and keeps the rest pending at QHEAD for the next frames.
    lda QLEN
    sec
    sbc QHEAD             ; entries still pending
    cmp #$20
    bcc nmi_have
    lda #$20
nmi_have:
    sta QOFF
    beq nmi_drained
    lda QHEAD
    asl a                 ; 2*QHEAD
    clc
    adc QHEAD             ; 3*QHEAD (<= 252: no page crossing)
    tax
    ldy #$00
nmi_drain:
    cpy QOFF
    bcs nmi_drained
    lda QUEUE0,x
    sta PPUADDR
    lda QUEUE1,x
    sta PPUADDR
    lda QUEUE2,x
    sta PPUDATA
    inx
    inx
    inx
    iny
    jmp nmi_drain
nmi_drained:
    lda QHEAD
    clc
    adc QOFF
    sta QHEAD
    cmp QLEN
    bcc nmi_keepq
    lda #$00             ; fully drained: the queue is empty again
    sta QLEN
    sta QHEAD
nmi_keepq:

    ; D-31: the canonical scroll reset after the frame's VRAM writes.
    ; Every queue drain writes $2006/$2007, which leaves the loopy t
    ; register pointing at the last written cell; without restoring
    ; it here the picture drifted a little every frame. The reset is
    ; the documented sequence: $2000 (nametable select) then two
    ; $2005 writes (0,0) - applied AFTER the last PPU write of the
    ; frame, so v reloads from a stable t at the pre-render line.
    lda #$80
    sta $2000
    lda #$00
    sta $2005
    sta $2005

    lda #$01
    sta NMIFLAG

    pla
    tay
    pla
    tax
    pla
    rti

irq:
    rti

; ---------------------------------------------------------------------------
wram_update:
    lda LEVEL
    sta TLVL
    lda LIVES
    sta TLIV
    lda SCOREL
    sta TSCR
    lda SCOREH
    sta TSCRH
    lda BANKWR
    sta TBWR
    lda DESTROY
    sta TBRK
    lda CURBANK
    sta TCBK
    rts

; ---------------------------------------------------------------------------
; data -----------------------------------------------------------------------
txt_ptrs:
    .dw txt_title, txt_sub, txt_press, txt_over, txt_win, txt_attract, chr_tiles, font_digits, font_letters

txt_title:   .db "SHEIKHEMU", 0
txt_sub:     .db "BREAKOUT", 0
txt_press:   .db "PRESS START", 0
txt_over:    .db "GAME OVER", 0
txt_win:     .db "YOU WIN", 0
txt_attract: .db "ATTRACT", 0
txt_level:   .db "LEVEL "

letter_map:
    .db $20,$21,$22,$00,$23,$00,$24,$25,$26,$00,$27,$28,$29,$2A,$2B,$2C
    .db $00,$2D,$2E,$2F,$30,$31,$32,$00,$33,$00

row_pat:
    .db $01,$01,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00
    .db $00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$00,$01,$01

pal_attract:
    ; D-31: the text ink is color 3; $28 is the white entry of this
    ; emulator's calibrated 2C02 table (its $30 renders pink - the
    ; table is a documented approximation; the ROM adapts, the core
    ; table is untouched).
    .db $0F,$10,$30,$28        ; bg palette 0: field, walls gray, text white (D-31)
    .db $0F,$16,$27,$16        ; bg palette 1: brick (red)
    .db $0F,$10,$30,$28        ; bg palette 2
    .db $0F,$10,$30,$28        ; bg palette 3
    .db $0F,$30,$21,$30        ; sprite palette 4: paddle
    .db $0F,$30,$16,$30        ; sprite palette 5: ball
    .db $0F,$30,$28,$30        ; sprite palette 6: lives
    .db $0F,$10,$30,$30        ; sprite palette 7

; D-31: 16-byte two-plane tiles. Plane 1 is zero for the scene tiles
; (single-plane ink = palette color 1); the life icon tile 7 carries
; the same pattern in both planes like the text glyphs (color 3).
chr_tiles:
    .db $00,$00,$00,$00,$00,$00,$00,$00          ; 0 blank (plane 0)
    .db $00,$00,$00,$00,$00,$00,$00,$00          ; 0 blank (plane 1)
    .db $FF,$81,$81,$81,$81,$81,$81,$FF          ; 1 wall (plane 0)
    .db $00,$00,$00,$00,$00,$00,$00,$00          ; 1 wall (plane 1)
    .db $00,$FF,$C3,$C3,$C3,$C3,$C3,$FF          ; 2 brick level 1 (plane 0)
    .db $00,$00,$00,$00,$00,$00,$00,$00          ; 2 brick level 1 (plane 1)
    .db $00,$FF,$AA,$55,$AA,$55,$AA,$FF          ; 3 brick level 2 (plane 0)
    .db $00,$00,$00,$00,$00,$00,$00,$00          ; 3 brick level 2 (plane 1)
    .db $00,$FF,$66,$99,$66,$99,$66,$FF          ; 4 brick level 3 (plane 0)
    .db $00,$00,$00,$00,$00,$00,$00,$00          ; 4 brick level 3 (plane 1)
    .db $00,$00,$FF,$FF,$FF,$FF,$00,$00          ; 5 paddle segment (plane 0)
    .db $00,$00,$00,$00,$00,$00,$00,$00          ; 5 paddle segment (plane 1)
    .db $00,$3C,$7E,$7E,$7E,$7E,$3C,$00          ; 6 ball (plane 0)
    .db $00,$00,$00,$00,$00,$00,$00,$00          ; 6 ball (plane 1)
    .db $00,$00,$18,$18,$18,$00,$00,$00          ; 7 life icon (plane 0)
    .db $00,$00,$18,$18,$18,$00,$00,$00          ; 7 life icon (plane 1: color 3)

font_digits:                  ; D-31: two-plane 16-byte tiles (ink = color 3)
    .db $38, $44, $44, $44, $44, $44, $38, $00          ; '0' plane 0
    .db $38, $44, $44, $44, $44, $44, $38, $00          ; '0' plane 1 (D-31: color 3)
    .db $10, $30, $10, $10, $10, $10, $38, $00          ; '1' plane 0
    .db $10, $30, $10, $10, $10, $10, $38, $00          ; '1' plane 1 (D-31: color 3)
    .db $38, $44, $04, $08, $10, $20, $7C, $00          ; '2' plane 0
    .db $38, $44, $04, $08, $10, $20, $7C, $00          ; '2' plane 1 (D-31: color 3)
    .db $38, $44, $04, $18, $04, $44, $38, $00          ; '3' plane 0
    .db $38, $44, $04, $18, $04, $44, $38, $00          ; '3' plane 1 (D-31: color 3)
    .db $08, $18, $28, $48, $7C, $08, $08, $00          ; '4' plane 0
    .db $08, $18, $28, $48, $7C, $08, $08, $00          ; '4' plane 1 (D-31: color 3)
    .db $7C, $40, $78, $04, $04, $44, $38, $00          ; '5' plane 0
    .db $7C, $40, $78, $04, $04, $44, $38, $00          ; '5' plane 1 (D-31: color 3)
    .db $38, $44, $40, $78, $44, $44, $38, $00          ; '6' plane 0
    .db $38, $44, $40, $78, $44, $44, $38, $00          ; '6' plane 1 (D-31: color 3)
    .db $7C, $04, $08, $10, $10, $10, $10, $00          ; '7' plane 0
    .db $7C, $04, $08, $10, $10, $10, $10, $00          ; '7' plane 1 (D-31: color 3)
    .db $38, $44, $44, $38, $44, $44, $38, $00          ; '8' plane 0
    .db $38, $44, $44, $38, $44, $44, $38, $00          ; '8' plane 1 (D-31: color 3)
    .db $38, $44, $44, $3C, $04, $44, $38, $00          ; '9' plane 0
    .db $38, $44, $44, $3C, $04, $44, $38, $00          ; '9' plane 1 (D-31: color 3)

font_letters:                 ; D-31: two-plane 16-byte tiles (ink = color 3)
    .db $38, $44, $44, $7C, $44, $44, $44, $00          ; 'A' plane 0
    .db $38, $44, $44, $7C, $44, $44, $44, $00          ; 'A' plane 1 (D-31: color 3)
    .db $78, $44, $44, $78, $44, $44, $78, $00          ; 'B' plane 0
    .db $78, $44, $44, $78, $44, $44, $78, $00          ; 'B' plane 1 (D-31: color 3)
    .db $38, $44, $40, $40, $40, $44, $38, $00          ; 'C' plane 0
    .db $38, $44, $40, $40, $40, $44, $38, $00          ; 'C' plane 1 (D-31: color 3)
    .db $7C, $40, $40, $78, $40, $40, $7C, $00          ; 'E' plane 0
    .db $7C, $40, $40, $78, $40, $40, $7C, $00          ; 'E' plane 1 (D-31: color 3)
    .db $38, $44, $40, $4C, $44, $44, $38, $00          ; 'G' plane 0
    .db $38, $44, $40, $4C, $44, $44, $38, $00          ; 'G' plane 1 (D-31: color 3)
    .db $44, $44, $44, $7C, $44, $44, $44, $00          ; 'H' plane 0
    .db $44, $44, $44, $7C, $44, $44, $44, $00          ; 'H' plane 1 (D-31: color 3)
    .db $38, $10, $10, $10, $10, $10, $38, $00          ; 'I' plane 0
    .db $38, $10, $10, $10, $10, $10, $38, $00          ; 'I' plane 1 (D-31: color 3)
    .db $44, $48, $50, $60, $50, $48, $44, $00          ; 'K' plane 0
    .db $44, $48, $50, $60, $50, $48, $44, $00          ; 'K' plane 1 (D-31: color 3)
    .db $40, $40, $40, $40, $40, $40, $7C, $00          ; 'L' plane 0
    .db $40, $40, $40, $40, $40, $40, $7C, $00          ; 'L' plane 1 (D-31: color 3)
    .db $44, $6C, $54, $44, $44, $44, $44, $00          ; 'M' plane 0
    .db $44, $6C, $54, $44, $44, $44, $44, $00          ; 'M' plane 1 (D-31: color 3)
    .db $44, $64, $54, $54, $4C, $44, $44, $00          ; 'N' plane 0
    .db $44, $64, $54, $54, $4C, $44, $44, $00          ; 'N' plane 1 (D-31: color 3)
    .db $38, $44, $44, $44, $44, $44, $38, $00          ; 'O' plane 0
    .db $38, $44, $44, $44, $44, $44, $38, $00          ; 'O' plane 1 (D-31: color 3)
    .db $78, $44, $44, $78, $40, $40, $40, $00          ; 'P' plane 0
    .db $78, $44, $44, $78, $40, $40, $40, $00          ; 'P' plane 1 (D-31: color 3)
    .db $78, $44, $44, $78, $50, $48, $44, $00          ; 'R' plane 0
    .db $78, $44, $44, $78, $50, $48, $44, $00          ; 'R' plane 1 (D-31: color 3)
    .db $3C, $40, $40, $38, $04, $04, $78, $00          ; 'S' plane 0
    .db $3C, $40, $40, $38, $04, $04, $78, $00          ; 'S' plane 1 (D-31: color 3)
    .db $7C, $10, $10, $10, $10, $10, $10, $00          ; 'T' plane 0
    .db $7C, $10, $10, $10, $10, $10, $10, $00          ; 'T' plane 1 (D-31: color 3)
    .db $44, $44, $44, $44, $44, $44, $38, $00          ; 'U' plane 0
    .db $44, $44, $44, $44, $44, $44, $38, $00          ; 'U' plane 1 (D-31: color 3)
    .db $44, $44, $44, $44, $44, $28, $10, $00          ; 'V' plane 0
    .db $44, $44, $44, $44, $44, $28, $10, $00          ; 'V' plane 1 (D-31: color 3)
    .db $44, $44, $44, $54, $54, $6C, $44, $00          ; 'W' plane 0
    .db $44, $44, $44, $54, $54, $6C, $44, $00          ; 'W' plane 1 (D-31: color 3)
    .db $44, $44, $44, $28, $10, $10, $10, $00          ; 'Y' plane 0
    .db $44, $44, $44, $28, $10, $10, $10, $00          ; 'Y' plane 1 (D-31: color 3)

.org $FFFA
    .dw nmi, reset, irq
