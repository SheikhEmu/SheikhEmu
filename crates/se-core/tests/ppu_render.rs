//! M2.2/M2.3 calibration (أ-style): render through the real register
//! interface and prove the produced pixels against hand-computed
//! palette indices.
//!
//! The cartridges below are 100% synthetic (JMP-self loops plus data
//! written through the `$2006/$2007` pipeline). Every expected pixel
//! is derived by hand from the loopy scroll model, the attribute
//! quadrant rule and the pattern bit numbering documented on the
//! NESdev Wiki — an independent arithmetic model, not a replay of the
//! engine's own math.

use se_core::machine::Machine;
use se_core::ppu::{Chr, Mirroring};
use se_core::timing::Region;

/// A self-looping NROM cartridge (JMP $C000) with CHR RAM.
fn machine() -> Machine {
    let mut prg = vec![0xEA; 16 * 1024];
    prg[0..3].copy_from_slice(&[0x4C, 0x00, 0xC0]);
    prg[0x3FFA..0x4000].copy_from_slice(&[0x00, 0xC0, 0x00, 0xC0, 0x00, 0xC0]);
    Machine::from_cartridge(
        prg,
        Chr::Ram(Box::new([0; 0x2000])),
        Mirroring::Vertical,
        false,
        Region::Ntsc,
    )
    .expect("prg")
}

/// Drive the register pair to point `v` at a PPU address and write a
/// byte through `$2007`.
fn ppu_write(machine: &mut Machine, addr: u16, value: u8) {
    use se_core::cpu::Bus as _;
    machine.bus.write(0x2006, ((addr >> 8) as u8) & 0x3F);
    machine.bus.write(0x2006, addr as u8);
    machine.bus.write(0x2007, value);
}

/// Fill all eight pattern rows of `tile`: `lo` on the low plane and
/// `hi` on the high plane (the 16-byte slot layout).
fn fill_tile(machine: &mut Machine, tile: u8, lo: u8, hi: u8) {
    let base = u16::from(tile) << 4;
    for row in 0..8u16 {
        ppu_write(machine, base + row, lo);
        ppu_write(machine, base + 8 + row, hi);
    }
}

/// Point v/t at the nametable-0 origin with no scrolling.
fn reset_scroll(machine: &mut Machine) {
    use se_core::cpu::Bus as _;
    machine.bus.write(0x2002, 0); // reset w
    machine.bus.write(0x2005, 0x00);
    machine.bus.write(0x2005, 0x00);
    machine.bus.write(0x2006, 0x00);
    machine.bus.write(0x2006, 0x00);
}

/// Palette RAM entries 0..15 hold their own index, so the plane value
/// directly identifies the palette entry under test.
fn fill_palette(machine: &mut Machine) {
    for i in 0..16u16 {
        ppu_write(machine, 0x3F00 + i, i as u8);
    }
}

#[test]
fn background_pixels_match_the_hand_computed_model() {
    let mut m = machine();
    m.reset();

    // Tile 1: every pixel opaque with pattern bits 11 (index 3).
    fill_tile(&mut m, 1, 0xFF, 0xFF);
    // Tile 2: even pixels index 1 (lo bit set, hi clear), odd 0.
    fill_tile(&mut m, 2, 0xAA, 0x00);

    // Nametable 0: rows 0 and 1 alternate tiles 1 and 2; row 2 is all
    // tile 1 (for the deep-row check).
    for x in 0..32u16 {
        let tile = if x % 2 == 0 { 1 } else { 2 };
        ppu_write(&mut m, 0x2000 + x, tile);
        ppu_write(&mut m, 0x2020 + x, tile);
        ppu_write(&mut m, 0x2040 + x, 1);
    }
    // Attribute byte at $23C0: bits 2-3 = 0b10, so the right attribute
    // column (tiles 2-3) uses palette 2 and everything else palette 0.
    ppu_write(&mut m, 0x23C0, 0x08);

    fill_palette(&mut m);

    reset_scroll(&mut m);
    // Background rendering on, leftmost 8 columns included.
    use se_core::cpu::Bus as _;
    m.bus.write(0x2001, 0x0A);
    // Two frames: the first settles the pipeline, the second is the
    // full deterministic render (verified via the published plane).
    m.run_frames(2).expect("loop");

    let plane = m.ppu().last_frame_plane();
    // Expected model, line 0 (y=0):
    //  - x=0: tile 1 pixel 0 -> pattern bits 11 -> index 3; quadrant
    //    (0,0) -> palette 0 -> entry (0<<2)|3 = 3.
    assert_eq!(plane[0], 3, "x=0 y=0: tile1, palette 0");
    assert_eq!(plane[1], 3);
    //  - x=8: tile 2 pixel 0 (bit 7 of 0xAA = 1, hi bit = 0) ->
    //    index 1 -> entry (0<<2)|1 = 1.
    assert_eq!(plane[8], 1, "x=8 y=0: tile2 even pixel");
    //  - x=9: tile 2 pixel 1 (bit 6 of 0xAA = 0) -> index 0 ->
    //    transparent -> backdrop entry 0.
    assert_eq!(plane[9], 0, "x=9 y=0: tile2 odd pixel is the backdrop");
    //  - x=16 enters the second attribute column: quadrant (1,0) uses
    //    bits 2-3 of 0x08 = 2 -> tile 1 index 3 -> entry (2<<2)|3 = 11.
    assert_eq!(plane[16], 11, "x=16 y=0: tile1, palette 2");
    // Line 8 (y=8) is inside the second attribute row: quadrant (0,1)
    // -> attribute bits 4-5 of 0x08 = 0 -> palette 0; x=0: tile 1
    // index 3 -> entry (0<<2)|3 = 3.
    assert_eq!(plane[8 * 256], 3, "x=0 y=8: tile1, palette 0");
    // Deeper rows sample deeper pattern rows: row 16 -> fine Y 0 of
    // nt row 2 (all tile 1) -> entry 3.
    assert_eq!(plane[16 * 256], 3, "x=0 y=16: tile1 row 2, palette 0");

    // Grayscale masks the entry to its monochrome bits. (Assert on a
    // fully rendered row: a single run_frames(1) starting mid-line
    // keeps row 0 stale from the previous frame.)
    m.bus.write(0x2001, 0x0B);
    m.run_frames(1).expect("loop");
    let plane = m.ppu().last_frame_plane();
    assert_eq!(plane[8 * 256], 3 & 0x30, "grayscale folds the entry");

    // Without the show-left-8 bit the first eight columns fall back
    // to the backdrop (documented clipping).
    m.bus.write(0x2001, 0x08);
    m.run_frames(1).expect("loop");
    let plane = m.ppu().last_frame_plane();
    assert_eq!(plane[8 * 256], 0, "the clipped column shows the backdrop");
    assert_eq!(plane[8 * 256 + 8], 1, "column 8 still renders");
}

#[test]
fn fine_x_scroll_shifts_the_sampled_tile_pixels() {
    let mut m = machine();
    m.reset();

    fill_tile(&mut m, 1, 0xFF, 0xFF);
    fill_tile(&mut m, 2, 0xAA, 0x00);
    for x in 0..32u16 {
        let tile = if x % 2 == 0 { 1 } else { 2 };
        ppu_write(&mut m, 0x2000 + x, tile);
    }
    fill_palette(&mut m);

    // fine X = 3: pixel 0 samples tile 0's bit 4 (0xFF -> index 3).
    use se_core::cpu::Bus as _;
    m.bus.write(0x2002, 0);
    m.bus.write(0x2005, 0x03);
    m.bus.write(0x2005, 0x00);
    m.bus.write(0x2006, 0x00);
    m.bus.write(0x2006, 0x00);
    m.bus.write(0x2001, 0x0A);
    m.run_frames(2).expect("loop");

    let plane = m.ppu().last_frame_plane();
    // x=0..4: bits 4..0 of tile 0 (all 1) -> entry 3 with palette 0.
    assert_eq!(plane[0], 3, "fine X shifts sampling inside the tile");
    assert_eq!(plane[4], 3);
    // x=5 is the tile boundary (8 - 3): the pixels enter tile 1
    // (column 1 -> tile 2), pixel 0 -> index 1 -> entry 1.
    assert_eq!(plane[5], 1, "the fine-X boundary enters tile 1");
    // x=6: tile 2 pixel 1 -> bit 6 = 0 -> backdrop.
    assert_eq!(plane[6], 0, "tile 2 pixel 1 is the backdrop");
    // x=13 (5+8): tile column 2 -> tile 1, pixel 0 -> entry 3.
    assert_eq!(plane[13], 3, "x=13 returns to tile 1 (column 2)");
}

#[test]
fn sprite_pixels_and_sprite_zero_hit_match_the_model() {
    let mut m = machine();
    m.reset();

    // Background: tile 1 everywhere opaque (index 3), palette 0.
    fill_tile(&mut m, 1, 0xFF, 0xFF);
    for y in 0..30u16 {
        for x in 0..32u16 {
            ppu_write(&mut m, 0x2000 + y * 32 + x, 1);
        }
    }
    // Sprite tile 3: every pixel opaque (index 3).
    fill_tile(&mut m, 3, 0xFF, 0xFF);
    fill_palette(&mut m);

    // OAM: sprite 0 at (x=4, y=8) with tile 3, palette 1, in front;
    // sprite 1 at (x=4, y=8) with tile 3, palette 2, behind.
    use se_core::cpu::Bus as _;
    let sprites = [
        (8u8, 3u8, 0x01u8, 4u8), // y, tile, attr, x
        (8u8, 3u8, 0x22u8, 4u8), // attr 0x22: palette 2 + behind
    ];
    for (i, (y, tile, attr, x)) in sprites.iter().enumerate() {
        let base = i as u16 * 4;
        m.bus.write(0x2003, base as u8);
        m.bus.write(0x2004, *y);
        m.bus.write(0x2004, *tile);
        m.bus.write(0x2004, *attr);
        m.bus.write(0x2004, *x);
    }

    reset_scroll(&mut m);
    m.bus.write(0x2001, 0x1E); // bg + sprites + both left-8 shown
    m.run_frames(2).expect("loop");

    let plane = m.ppu().last_frame_plane();
    // The sprite (y=8) occupies rows 9..16, columns 4..12 (the OAM Y
    // byte is top-minus-one). Sprite 0 (palette 1) wins by OAM order;
    // sprite pixel index 3 -> entry (1<<2)|3 = 7.
    assert_eq!(plane[9 * 256 + 4], 7, "sprite 0 pixel at its top-left");
    assert_eq!(plane[9 * 256 + 11], 7, "sprite 0 pixel at its right edge");
    // Outside the sprite: background entry 3.
    assert_eq!(plane[9 * 256 + 3], 3, "background left of the sprite");
    assert_eq!(plane[17 * 256 + 4], 3, "background below the sprite");
    // The sprite-0 hit flag is transient (cleared at the pre-render
    // dot 1): observe it mid-frame, well past the sprite's rows.
    m.run_cycles(m.cycle() + 8000).expect("loop");
    assert!(m.ppu().status_bits() & 0x40 != 0, "sprite 0 hit");
}

#[test]
fn oam_dma_fills_the_sprites_and_renders_them() {
    let mut m = machine();
    m.reset();

    // Background tile 1 opaque; sprite tile 3 opaque.
    fill_tile(&mut m, 1, 0xFF, 0xFF);
    fill_tile(&mut m, 3, 0xFF, 0xFF);
    for y in 0..30u16 {
        for x in 0..32u16 {
            ppu_write(&mut m, 0x2000 + y * 32 + x, 1);
        }
    }
    fill_palette(&mut m);

    // Fill CPU page 2 with one sprite entry: y=8, tile=3, attr=0
    // (palette 0), x=4.
    use se_core::cpu::Bus as _;
    m.bus.write(0x0200, 8);
    m.bus.write(0x0201, 3);
    m.bus.write(0x0202, 0);
    m.bus.write(0x0203, 4);
    // Launch the DMA from page 2.
    m.bus.write(0x4014, 0x02);

    reset_scroll(&mut m);
    m.bus.write(0x2001, 0x1E);
    m.run_frames(2).expect("loop");

    // Sprite palette 0, index 3 -> entry 3 (same value as the
    // background — so prove the DMA delivered the sprite via the
    // sprite-0 hit flag instead).
    assert_eq!(
        m.ppu().last_frame_plane()[9 * 256 + 4],
        3,
        "the DMA-loaded sprite pixel"
    );
    // The hit flag is transient: observe it mid-frame.
    m.run_cycles(m.cycle() + 8000).expect("loop");
    assert!(
        m.ppu().status_bits() & 0x40 != 0,
        "the DMA-loaded sprite hit"
    );
    // The plane stayed deterministic.
    let first = m.ppu().last_plane_hash();
    m.run_frames(1).expect("loop");
    let second = m.ppu().last_plane_hash();
    // A static scene with sprite-0 already set: the next frame is
    // byte-identical.
    assert_eq!(first, second, "static scene renders identically");
}

#[test]
fn eight_by_16_sprites_select_the_two_planes() {
    let mut m = machine();
    m.reset();

    // Background tile 1 opaque.
    fill_tile(&mut m, 1, 0xFF, 0xFF);
    for y in 0..30u16 {
        for x in 0..32u16 {
            ppu_write(&mut m, 0x2000 + y * 32 + x, 1);
        }
    }
    // 8x16 sprite with tile index 2: the top 8 rows come from tile 2
    // in table 0, the bottom 8 rows from tile 3 in table 0 (even
    // base). Tile 2: only pixel column 0 opaque (index 1); tile 3:
    // only column 7 opaque (index 2).
    fill_tile(&mut m, 2, 0x80, 0x00);
    fill_tile(&mut m, 3, 0x00, 0x01);
    fill_palette(&mut m);

    // Sprite 0: y=0, tile=2 (8x16 -> planes 2 and 3), attr palette 1,
    // x=0.
    use se_core::cpu::Bus as _;
    m.bus.write(0x2003, 0);
    m.bus.write(0x2004, 0);
    m.bus.write(0x2004, 2);
    m.bus.write(0x2004, 0x01);
    m.bus.write(0x2004, 0);
    // 8x16 mode.
    m.bus.write(0x2000, 0x20);
    reset_scroll(&mut m);
    m.bus.write(0x2001, 0x1E);
    m.run_frames(2).expect("loop");

    let plane = m.ppu().last_frame_plane();
    // Sprite 0 with y=0 paints rows 1..16 (top-minus-one): tile 2 in
    // rows 1..8, tile 3 in rows 9..16.
    // Row 1: tile 2 pixel 0 -> index 1 -> entry (1<<2)|1 = 5; pixel 1
    // -> 0 (background entry 3).
    assert_eq!(plane[256], 5, "8x16 top plane pixel 0");
    assert_eq!(plane[256 + 1], 3, "8x16 top plane pixel 1 falls to bg");
    // Row 9: tile 3 pixel 7 -> index 2 -> entry (1<<2)|2 = 6.
    assert_eq!(plane[9 * 256 + 7], 6, "8x16 bottom plane pixel 7");
    assert_eq!(
        plane[9 * 256 + 6],
        3,
        "8x16 bottom plane pixel 6 falls to bg"
    );
}
