//! Picture Processing Unit (RP2C02 family, B1 scope).
//!
//! Implements the M2 milestone of the spec (§5.3, level B1): the
//! 341-dot scanline grid per region, the `$2000-$2007` registers with
//! the loopy `v/t/x/w` scroll model, nametable VRAM with header
//! mirroring, palette memory with the `$3F1x` mirror rule, OAM, the
//! background pipeline with fine scrolling, sprite evaluation with the
//! sprite-0 hit, VBlank/NMI signaling with the documented suppression
//! windows, and the NTSC odd-frame dot skip.
//!
//! Sources: NESdev Wiki "PPU rendering", "PPU registers", "PPU
//! scrolling", "PPU frame timing", "PPU OAM", "PPU palettes" and the
//! 2C02 palette data published there (public documented data, not
//! code).
//!
//! Two documented B1 simplifications (see `docs/PRECISION_BACKLOG.md`
//! for the deferred B2 refinements):
//!
//! * Background rendering takes one *scroll snapshot* of the loopy
//!   `v` register at the start of each visible scanline and derives
//!   every pixel of that line from it. Register writes therefore take
//!   effect at scanline granularity — exactly what line-granular
//!   splits (status bars) need — while the hardware's two-tile
//!   shifter delay for sub-scanline writes is a B2 refinement.
//! * The sprite-overflow flag uses the simple "more than eight"
//!   model, not the buggy hardware sequence.

use crate::timing::{Region, DOTS_PER_SCANLINE};

/// Nametable mirroring of the cartridge (physical layout of the VRAM
/// across the four `$2000-$2FFF` slots).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mirroring {
    /// Vertical mirroring (iNES flag bit 0 clear): the two unique
    /// tables sit side by side; `$2000/$2800` share table A and
    /// `$2400/$2C00` share table B.
    Vertical,
    /// Horizontal mirroring (iNES flag bit 0 set): the two unique
    /// tables are stacked; `$2000/$2400` share table A and
    /// `$2800/$2C00` share table B.
    Horizontal,
    /// Four-screen VRAM (iNES flag bit 3 set): 4 KiB of independent
    /// nametables.
    FourScreen,
}

/// Pattern-table memory: CHR-ROM from the cartridge or CHR-RAM.
#[derive(Clone, Debug)]
pub enum Chr {
    /// Read-only CHR ROM of arbitrary size (addressed modulo its
    /// length).
    Rom(Vec<u8>),
    /// Writable 8 KiB CHR RAM (used when the cartridge ships no CHR
    /// ROM banks).
    Ram(Box<[u8; 0x2000]>),
}

impl Chr {
    /// Byte at a `$0000-$1FFF` PPU address (modulo the ROM length).
    pub fn read(&self, addr: u16) -> u8 {
        let addr = addr & 0x1FFF;
        match self {
            Chr::Rom(rom) => rom[usize::from(addr) % rom.len().max(1)],
            Chr::Ram(ram) => ram[usize::from(addr)],
        }
    }

    /// Write a byte; CHR ROM ignores writes, CHR RAM accepts them.
    pub fn write(&mut self, addr: u16, value: u8) {
        if let Chr::Ram(ram) = self {
            ram[usize::from(addr & 0x1FFF)] = value;
        }
    }

    /// The CHR RAM bytes when the cartridge uses RAM (state hashing).
    pub fn ram_bytes(&self) -> Option<&[u8; 0x2000]> {
        match self {
            Chr::Rom(_) => None,
            Chr::Ram(ram) => Some(ram),
        }
    }
}

/// The documented 2C02 palette (NESdev Wiki "PPU palettes"), 64 RGB
/// entries. Public domain reference data.
pub const PALETTE_2C02: [u32; 64] = [
    0x525252, 0x001E88, 0x14012A, 0x3B0E74, 0x5C0F6E, 0x6E1160, 0x6C141F, 0x562009, 0x335507,
    0x0B5E01, 0x0A6000, 0x034F0A, 0x003D4B, 0x000000, 0x000000, 0x000000, 0x9C9C9C, 0x1963C5,
    0x4280FF, 0x7435FF, 0x9C27FF, 0xBC27C5, 0xD42C54, 0xE43C1D, 0xDB5100, 0xC98000, 0xA8A200,
    0x819B00, 0x5D9500, 0x0A8700, 0x1E8B00, 0x008355, 0x007B7B, 0x0077B5, 0x0071B5, 0x1E6A9E,
    0x000000, 0x000000, 0x000000, 0x000000, 0xFFFFFF, 0x64B0FF, 0x9CFFFF, 0xD4FFFF, 0xE3FFFF,
    0xFFEFFF, 0xFFE3DF, 0xFFC3C3, 0xFFABAB, 0xFF9B97, 0xFF9470, 0xFB8038, 0xE34800, 0xC40000,
    0x9C0000, 0x6E0000, 0x000000, 0x000000, 0x000000, 0x000000, 0xFFFFFF, 0xFFFFFF, 0xFFFFFF,
    0xFFFFFF,
];

/// Scanline that raises VBlank (dot 1) in every region.
const VBLANK_SCANLINE: u16 = 241;

/// The suppression windows of the `$2002` race, in PPU dots.
///
/// The dot at which the vblank flag sets is `S` and a read of `$2002`
/// is serviced while the PPU sits at position `P` (the next dot to
/// tick, i.e. exactly the dots the read can still see happen).
///
/// * `P <= S <= P + SUPPRESS_BEFORE_DOTS` (read zero to two CPU
///   cycles at/around the set): the read returns 0 and the set is
///   swallowed entirely — the flag never appears this frame and the
///   NMI edge never fires.
/// * `S < P <= S + SUPPRESS_AFTER_DOTS` (read up to one CPU cycle
///   after the set): the read returns 1 but the pending NMI is
///   suppressed.
/// * Later reads: flag 1, NMI untouched.
const SUPPRESS_BEFORE_DOTS: u32 = 8;
const SUPPRESS_AFTER_DOTS: u32 = 3;

/// The PPU. Owns VRAM, palette, OAM and the pixel planes.
#[derive(Clone, Debug)]
pub struct Ppu {
    // ---- region ------------------------------------------------------
    region: Region,

    // ---- CPU-facing registers ----------------------------------------
    /// `$2000` PPUCTRL (bit 7 = NMI enable).
    pub ctrl: u8,
    /// `$2001` PPUMASK (rendering enables, grayscale, emphasis).
    pub mask: u8,
    vblank: bool,
    sprite_zero: bool,
    overflow: bool,
    /// `$2003` OAMADDR.
    oam_addr: u8,
    /// The `$2007` read buffer.
    read_buffer: u8,

    // ---- loopy scroll model --------------------------------------------
    v: u16,
    t: u16,
    fine_x: u8,
    w: bool,

    // ---- memories ------------------------------------------------------
    vram: Vec<u8>,
    mirroring: Mirroring,
    palette: [u8; 0x20],
    oam: [u8; 256],
    chr: Chr,

    // ---- timing ----------------------------------------------------------
    scanline: u16,
    dot: u16,
    frame: u64,
    odd_frame: bool,

    // ---- vblank race state -----------------------------------------------
    /// The frame dot at which the current frame's flag set, if it did.
    vbl_set_dot: Option<u32>,
    /// A read swallowed the upcoming set; the flag must not appear.
    swallow_set: bool,

    // ---- interrupt state -------------------------------------------------
    /// The latched "take an NMI at the next instruction boundary"
    /// signal (edge model; cleared by suppressing reads).
    nmi_pending: bool,

    // ---- per-line rendering state ------------------------------------------
    /// The scroll snapshot (a copy of `v`) of the current visible line.
    line_v: u16,
    /// Sprite slots evaluated for the current line.
    sprites: [SpriteSlot; 8],
    sprites_in_line: u8,
    sprite_zero_in_line: bool,

    // ---- output -------------------------------------------------------------
    /// Palette-index plane of the frame in progress.
    plane: Box<[u8; 256 * 240]>,
    /// The plane of the last completed frame (published at frame end).
    last_plane: Box<[u8; 256 * 240]>,
}

/// One evaluated sprite slot (secondary OAM equivalent).
#[derive(Clone, Copy, Debug, Default)]
struct SpriteSlot {
    is_sprite_zero: bool,
    attr: u8,
    lo: u8,
    hi: u8,
    x: u8,
}

fn ctrl_bg_table(ctrl: u8) -> u16 {
    u16::from(ctrl & 0x08 != 0) << 12
}
fn ctrl_sprite_table(ctrl: u8) -> u16 {
    u16::from(ctrl & 0x04 != 0) << 12
}
fn ctrl_tall_sprites(ctrl: u8) -> bool {
    ctrl & 0x20 != 0
}
fn ctrl_nmi_enabled(ctrl: u8) -> bool {
    ctrl & 0x80 != 0
}

fn mask_rendering(mask: u8) -> bool {
    mask & 0x18 != 0
}
fn mask_bg_enabled(mask: u8) -> bool {
    mask & 0x08 != 0
}
fn mask_sprites_enabled(mask: u8) -> bool {
    mask & 0x10 != 0
}
fn mask_bg_left_enabled(mask: u8) -> bool {
    mask & 0x02 != 0
}
fn mask_sprites_left_enabled(mask: u8) -> bool {
    mask & 0x04 != 0
}
fn mask_grayscale(mask: u8) -> bool {
    mask & 0x01 != 0
}

impl Ppu {
    /// Build a PPU for a region, CHR memory and mirroring.
    pub fn new(region: Region, chr: Chr, mirroring: Mirroring) -> Self {
        let vram_len = if mirroring == Mirroring::FourScreen {
            0x1000
        } else {
            0x0800
        };
        Ppu {
            region,
            ctrl: 0,
            mask: 0,
            vblank: false,
            sprite_zero: false,
            overflow: false,
            oam_addr: 0,
            read_buffer: 0,
            v: 0,
            t: 0,
            fine_x: 0,
            w: false,
            vram: vec![0; vram_len],
            mirroring,
            palette: [0; 0x20],
            oam: [0; 256],
            chr,
            scanline: 0,
            dot: 0,
            frame: 0,
            odd_frame: false,
            vbl_set_dot: None,
            swallow_set: false,
            nmi_pending: false,
            line_v: 0,
            sprites: [SpriteSlot::default(); 8],
            sprites_in_line: 0,
            sprite_zero_in_line: false,
            plane: Box::new([0; 256 * 240]),
            last_plane: Box::new([0; 256 * 240]),
        }
    }

    // ---- timing ---------------------------------------------------------

    /// Current `(scanline, dot)` position (the next dot to tick).
    pub fn position(&self) -> (u16, u16) {
        (self.scanline, self.dot)
    }

    /// Completed frame count.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// The NTSC odd/even frame parity.
    pub fn odd_frame(&self) -> bool {
        self.odd_frame
    }

    /// Whether rendering is currently switched on (`$2001` bits 3/4).
    pub fn rendering_enabled(&self) -> bool {
        mask_rendering(self.mask)
    }

    /// Dots processed inside the current frame (the position as a
    /// frame-relative dot index).
    fn frame_dot_index(&self) -> u32 {
        u32::from(self.scanline) * DOTS_PER_SCANLINE + u32::from(self.dot)
    }

    /// The dot index at which the vblank flag sets each frame.
    fn vblank_set_index(&self) -> u32 {
        u32::from(VBLANK_SCANLINE) * DOTS_PER_SCANLINE + 1
    }

    fn visible(&self) -> bool {
        self.scanline < 240
    }

    fn pre_render_line(&self) -> bool {
        self.scanline == self.scanlines() - 1
    }

    fn scanlines(&self) -> u16 {
        self.region.scanlines_per_frame() as u16
    }

    /// Advance exactly one dot. The whole frame state machine.
    pub fn tick(&mut self) {
        // ---- event flags at dot 1 --------------------------------------
        if self.dot == 1 {
            if self.scanline == VBLANK_SCANLINE {
                if self.swallow_set {
                    self.swallow_set = false;
                } else {
                    self.vblank = true;
                    self.vbl_set_dot = Some(self.frame_dot_index());
                    if ctrl_nmi_enabled(self.ctrl) {
                        self.nmi_pending = true;
                    }
                }
            } else if self.pre_render_line() {
                self.vblank = false;
                self.sprite_zero = false;
                self.overflow = false;
                self.vbl_set_dot = None;
                self.swallow_set = false;
            }
        }

        // ---- the rendering window ----------------------------------------
        if mask_rendering(self.mask) && (self.visible() || self.pre_render_line()) {
            self.tick_rendering();
        }

        // ---- dot advance ----------------------------------------------
        self.dot += 1;
        let skip = self.region == Region::Ntsc
            && self.pre_render_line()
            && self.dot == 340
            && self.odd_frame
            && mask_bg_enabled(self.mask);
        if self.dot >= DOTS_PER_SCANLINE as u16 || skip {
            self.dot = 0;
            self.scanline += 1;
            if self.scanline >= self.scanlines() {
                self.scanline = 0;
                self.frame += 1;
                self.odd_frame = !self.odd_frame;
                core::mem::swap(&mut self.plane, &mut self.last_plane);
            }
        }
    }

    /// The rendering portion of one dot: scroll snapshot, the
    /// documented `v` increment cadence, sprite evaluation and pixel
    /// output.
    fn tick_rendering(&mut self) {
        // Scroll snapshot for the *next* line, taken at dot 320: after
        // the horizontal restore (dot 257) and the vertical restore
        // (pre-render dots 280-304) but before the two-tile prefetch
        // of dots 321-336 (which advances the fetch pointer `v` past
        // the tiles the shifter pipeline will display). This is the
        // B1 line-granular split model; see the module docs.
        if self.dot == 320 {
            self.line_v = self.v;
        }

        // OAMADDR is cleared during the sprite fetch phase.
        if self.dot >= 257 && self.dot <= 320 {
            self.oam_addr = 0;
        }

        // The horizontal fetch cadence moves `v` across the line every
        // eight dots (during 1..=256 and the 321..=336 prefetch).
        if ((1..=256).contains(&self.dot) || (321..=336).contains(&self.dot))
            && self.dot.is_multiple_of(8)
        {
            self.increment_coarse_x();
        }

        // Vertical increment at the last fetch of the line.
        if self.dot == 256 {
            self.increment_y();
        }

        // Restore the horizontal bits from t at the sprite phase.
        if self.dot == 257 {
            self.v = (self.v & !0x041F) | (self.t & 0x041F);
        }

        // Restore the vertical bits from t on the pre-render line.
        if self.pre_render_line() && (280..=304).contains(&self.dot) {
            self.v = (self.v & !0x7BE0) | (self.t & 0x7BE0);
        }

        // Sprite evaluation for the *next* line happens at dot 65.
        if self.visible() && self.dot == 65 {
            self.evaluate_sprites();
        }

        // Pixel output at dots 2..=257 of visible lines.
        if self.visible() && self.dot >= 2 && self.dot <= 257 {
            self.output_pixel();
        }
    }

    /// Increment the coarse X scroll, flipping nametable X on wrap.
    fn increment_coarse_x(&mut self) {
        if (self.v & 0x001F) == 31 {
            self.v &= !0x001F;
            self.v ^= 0x0400;
        } else {
            self.v += 1;
        }
    }

    /// Increment the vertical scroll (fine Y, then coarse Y with
    /// nametable-Y flip; coarse Y 29 wraps while 30/31 do not).
    fn increment_y(&mut self) {
        if (self.v & 0x7000) != 0x7000 {
            self.v += 0x1000;
        } else {
            self.v &= !0x7000;
            let y = (self.v & 0x03E0) >> 5;
            if y == 29 {
                self.v &= !0x03E0;
                self.v ^= 0x0800;
            } else if y == 31 {
                self.v &= !0x03E0;
            } else {
                self.v += 0x0020;
            }
        }
    }

    /// Sprite evaluation for the next scanline (B1 model: one pass at
    /// dot 65; the exact sub-dot sequence is a B2 item).
    ///
    /// The OAM Y byte stores the sprite top *minus one* (the
    /// documented hardware convention): a sprite with y=8 paints
    /// rows 9..16.
    fn evaluate_sprites(&mut self) {
        let tall = ctrl_tall_sprites(self.ctrl);
        let next_line = ((self.scanline + 1) % self.scanlines()) as u8;
        let mut count = 0u8;
        let mut zero = false;
        let mut overflow_seen = false;
        for oam in 0..64usize {
            let y = self.oam[oam * 4];
            let height: u8 = if tall { 16 } else { 8 };
            // In-range test with the minus-one convention: the sprite
            // covers rows y+1 .. y+height.
            let top = y.wrapping_add(1);
            let dy = next_line.wrapping_sub(top);
            if dy < height {
                if count < 8 {
                    let attr = self.oam[oam * 4 + 2];
                    let row = dy;
                    let row_eff = if attr & 0x80 != 0 {
                        height - 1 - row
                    } else {
                        row
                    };
                    let tile = self.oam[oam * 4 + 1];
                    let (base, table) = if tall {
                        // 8x16: the low bit of the tile selects the
                        // pattern table; the even base tile plus the
                        // row's 8-row half selects the plane.
                        ((tile & 0xFE) | (row_eff >> 3), u16::from(tile & 0x01) << 12)
                    } else {
                        (tile, ctrl_sprite_table(self.ctrl))
                    };
                    let addr = table | (u16::from(base) << 4) | u16::from(row_eff & 0x07);
                    let lo = self.chr.read(addr);
                    let hi = self.chr.read(addr | 0x0008);
                    let slot = &mut self.sprites[usize::from(count)];
                    slot.is_sprite_zero = oam == 0;
                    slot.attr = attr;
                    slot.lo = if attr & 0x40 != 0 {
                        lo.reverse_bits()
                    } else {
                        lo
                    };
                    slot.hi = if attr & 0x40 != 0 {
                        hi.reverse_bits()
                    } else {
                        hi
                    };
                    slot.x = self.oam[oam * 4 + 3];
                    if oam == 0 {
                        zero = true;
                    }
                    count += 1;
                } else {
                    overflow_seen = true;
                }
            }
        }
        for slot in self.sprites[usize::from(count)..].iter_mut() {
            *slot = SpriteSlot::default();
        }
        self.sprites_in_line = count;
        self.sprite_zero_in_line = zero;
        if overflow_seen {
            self.overflow = true;
        }
    }

    /// Produce one pixel of output for screen column `x = dot - 2`.
    fn output_pixel(&mut self) {
        let x = (self.dot - 2) as usize;
        let y = self.scanline as usize;
        let bg_showing = mask_bg_enabled(self.mask) && (mask_bg_left_enabled(self.mask) || x >= 8);
        let spr_showing =
            mask_sprites_enabled(self.mask) && (mask_sprites_left_enabled(self.mask) || x >= 8);

        let (bg_index, bg_palette) = if bg_showing {
            self.bg_pixel(x)
        } else {
            (0u8, 0u8)
        };

        let mut spr: Option<(u8, bool, bool)> = None; // (entry, behind, is zero)
        if spr_showing {
            for i in 0..usize::from(self.sprites_in_line) {
                let slot = &self.sprites[i];
                if x >= usize::from(slot.x) && x - usize::from(slot.x) < 8 {
                    let shift = 7 - (x - usize::from(slot.x));
                    let lo = (slot.lo >> shift) & 1;
                    let hi = (slot.hi >> shift) & 1;
                    let index = (hi << 1) | lo;
                    if index != 0 {
                        let palette = (slot.attr & 0x03) | 0x10;
                        spr = Some((
                            (palette << 2) | index,
                            slot.attr & 0x20 != 0,
                            slot.is_sprite_zero,
                        ));
                        break;
                    }
                }
            }
        }

        // Sprite-0 hit: opaque sprite-0 pixel over an opaque
        // background pixel, both layers enabled, x < 255.
        if let Some((_, _, true)) = spr {
            if bg_index != 0 && bg_showing && x < 255 {
                self.sprite_zero = true;
            }
        }

        let entry = match (bg_index != 0, spr) {
            (false, None) => 0,
            (false, Some((s, _, _))) => s,
            (true, None) => (bg_palette << 2) | bg_index,
            (true, Some((s, behind, _))) => {
                if behind {
                    (bg_palette << 2) | bg_index
                } else {
                    s
                }
            }
        };
        let mut plane = entry & 0x1F;
        if mask_grayscale(self.mask) {
            plane &= 0x30;
        }
        self.plane[y * 256 + x] = plane;
    }

    /// Background pixel for column `x` from the line scroll snapshot.
    ///
    /// Returns `(pattern bits 0..3, attribute palette 0..3)`.
    fn bg_pixel(&self, x: usize) -> (u8, u8) {
        let fine_x = u32::from(self.fine_x);
        let column = fine_x + x as u32; // the nametable column 0..262
        let tile_offset = column >> 3; // tiles across, 0..33
        let bit = 7 - (column & 7);

        let snap = self.line_v;
        let coarse_x = u32::from(snap & 0x001F);
        let coarse_y = u32::from((snap >> 5) & 0x001F);
        let fine_y = u32::from((snap >> 12) & 0x0007);
        let nt_x = u32::from((snap >> 10) & 1);
        let nt_y = u32::from((snap >> 11) & 1);

        let total_x = coarse_x + tile_offset;
        let nt_x_eff = nt_x ^ (total_x >> 5);
        let tile_x = total_x & 0x1F;

        // Nametable byte.
        let nt_addr = 0x2000u32 | (nt_y << 11) | (nt_x_eff << 10) | (coarse_y << 5) | tile_x;
        let tile = self.vram_read(nt_addr as u16);

        // Attribute byte and its quadrant.
        let at_addr =
            0x23C0u32 | (nt_y << 11) | (nt_x_eff << 10) | ((coarse_y >> 2) << 3) | (tile_x >> 2);
        let at = self.vram_read(at_addr as u16);
        let shift = ((coarse_y & 0x02) << 1) | (tile_x & 0x02);
        let palette = (at >> shift) & 0x03;

        // Pattern bits.
        let addr = ctrl_bg_table(self.ctrl) | ((tile as u16) << 4) | fine_y as u16;
        let lo = self.chr.read(addr);
        let hi = self.chr.read(addr | 0x0008);
        let index = ((u32::from((hi >> bit) & 1) << 1) | u32::from((lo >> bit) & 1)) as u8;
        (index, palette)
    }

    // ---- CPU register interface ------------------------------------------

    /// `$2000` write.
    pub fn write_ctrl(&mut self, value: u8) {
        self.t = (self.t & !0x0C00) | ((u16::from(value) & 0x03) << 10);
        self.ctrl = value;
        // Enabling NMI while vblank is active re-triggers the edge.
        if ctrl_nmi_enabled(value) && self.vblank {
            self.nmi_pending = true;
        }
    }

    /// `$2001` write.
    pub fn write_mask(&mut self, value: u8) {
        self.mask = value;
    }

    /// `$2002` read. `open_bus` supplies the low five bits the CPU
    /// data bus would show (level B1 documented behavior).
    pub fn read_status(&mut self, open_bus: u8) -> u8 {
        let value = u8::from(self.vblank) << 7
            | u8::from(self.sprite_zero) << 6
            | u8::from(self.overflow) << 5
            | (open_bus & 0x1F);

        let pos = self.frame_dot_index();
        match self.vbl_set_dot {
            // The flag already set this frame.
            Some(set_dot) => {
                if pos <= set_dot + SUPPRESS_AFTER_DOTS {
                    // Read within one CPU cycle after the set: the
                    // pending NMI is suppressed.
                    self.nmi_pending = false;
                }
            }
            // The set is still ahead: a read close enough swallows it.
            None => {
                let set_dot = self.vblank_set_index();
                if pos <= set_dot && set_dot <= pos + SUPPRESS_BEFORE_DOTS {
                    self.swallow_set = true;
                    self.nmi_pending = false;
                }
            }
        }
        // Reading the register always clears the flag and w.
        self.vblank = false;
        self.w = false;
        value
    }

    /// `$2003` write.
    pub fn write_oam_addr(&mut self, value: u8) {
        self.oam_addr = value;
    }

    /// `$2004` read.
    pub fn read_oam(&mut self) -> u8 {
        let value = if self.rendering_enabled() {
            // B1 simplification: reads during rendering return $FF
            // (documented simplification; the precise behavior is a
            // B2 item).
            0xFF
        } else {
            self.oam[usize::from(self.oam_addr)]
        };
        self.oam_addr = self.oam_addr.wrapping_add(1);
        value
    }

    /// `$2004` write.
    pub fn write_oam_data(&mut self, value: u8) {
        if self.rendering_enabled() {
            // B1: writes during rendering are ignored (the corrupted
            // write behavior is B2).
            return;
        }
        self.oam[usize::from(self.oam_addr)] = value;
        self.oam_addr = self.oam_addr.wrapping_add(1);
    }

    /// `$2005` write.
    pub fn write_scroll(&mut self, value: u8) {
        if !self.w {
            self.fine_x = value & 0x07;
            self.t = (self.t & !0x001F) | (u16::from(value) >> 3);
            self.w = true;
        } else {
            self.t = (self.t & !0x73E0)
                | (u16::from(value & 0x07) << 12)
                | (u16::from(value & 0xF8) << 2);
            self.w = false;
        }
    }

    /// `$2006` write.
    pub fn write_addr(&mut self, value: u8) {
        if !self.w {
            self.t = (self.t & 0x00FF) | (u16::from(value & 0x3F) << 8);
            self.w = true;
        } else {
            self.t = (self.t & 0x7F00) | u16::from(value);
            self.v = self.t;
            self.w = false;
        }
    }

    /// `$2007` read (buffered; the palette range is immediate).
    pub fn read_data(&mut self) -> u8 {
        let addr = self.v & 0x3FFF;
        let value = if addr >= 0x3F00 {
            self.palette_read(addr)
        } else {
            let buffered = self.read_buffer;
            self.read_buffer = self.vram_read(addr);
            buffered
        };
        self.increment_vram_addr();
        value
    }

    /// `$2007` write.
    pub fn write_data(&mut self, value: u8) {
        let addr = self.v & 0x3FFF;
        if addr >= 0x3F00 {
            self.palette_write(addr, value);
        } else if addr < 0x2000 {
            self.chr.write(addr, value);
        } else {
            let index = self.vram_index(addr);
            self.vram[index] = value;
        }
        self.increment_vram_addr();
    }

    /// Increment v by the `$2000` bit-2 step (1 or 32).
    fn increment_vram_addr(&mut self) {
        let step: u16 = if self.ctrl & 0x04 != 0 { 32 } else { 1 };
        self.v = self.v.wrapping_add(step) & 0x7FFF;
    }

    /// Take the pending NMI edge (the CPU consumes it at an
    /// instruction boundary).
    pub fn take_nmi(&mut self) -> bool {
        if self.nmi_pending {
            self.nmi_pending = false;
            true
        } else {
            false
        }
    }

    /// Peek the pending NMI edge without consuming it.
    pub fn nmi_pending(&self) -> bool {
        self.nmi_pending
    }

    // ---- memory ------------------------------------------------------------

    /// Read VRAM through the mirroring at a PPU address.
    fn vram_read(&self, addr: u16) -> u8 {
        let addr = addr & 0x2FFF; // $3000-$3EFF mirrors $2000-$2EFF
        if addr < 0x2000 {
            self.chr.read(addr)
        } else {
            let index = self.vram_index(addr);
            self.vram[index]
        }
    }

    /// Physical VRAM index for a nametable address.
    fn vram_index(&self, addr: u16) -> usize {
        let offset = usize::from(addr & 0x0FFF);
        match self.mirroring {
            Mirroring::Vertical => offset & 0x07FF,
            Mirroring::Horizontal => {
                if offset & 0x0800 == 0 {
                    offset & 0x03FF
                } else {
                    0x0400 | (offset & 0x03FF)
                }
            }
            Mirroring::FourScreen => offset,
        }
    }

    /// Palette read with the `$3F1x` mirror rule.
    fn palette_read(&self, addr: u16) -> u8 {
        self.palette[Self::palette_index(addr)]
    }

    /// Palette write with the mirror rule.
    fn palette_write(&mut self, addr: u16, value: u8) {
        let index = Self::palette_index(addr);
        self.palette[index] = value;
    }

    fn palette_index(addr: u16) -> usize {
        let mut index = usize::from(addr & 0x1F);
        // $3F10/$14/$18/$1C mirror $3F00/$04/$08/$0C.
        if index & 0x13 == 0x10 {
            index &= !0x10;
        }
        index
    }

    // ---- state exposure (machine hashing and the CLI) ----------------------

    /// The loopy `v` register.
    pub fn scroll_v(&self) -> u16 {
        self.v
    }

    /// The loopy `t` register.
    pub fn scroll_t(&self) -> u16 {
        self.t
    }

    /// The fine X scroll.
    pub fn fine_x(&self) -> u8 {
        self.fine_x
    }

    /// The first/second write toggle.
    pub fn write_toggle(&self) -> bool {
        self.w
    }

    /// The OAM address register.
    pub fn oam_addr(&self) -> u8 {
        self.oam_addr
    }

    /// The `$2007` read buffer.
    pub fn read_buffer(&self) -> u8 {
        self.read_buffer
    }

    /// The status bits packed as `$2002` would show them.
    pub fn status_bits(&self) -> u8 {
        u8::from(self.vblank) << 7 | u8::from(self.sprite_zero) << 6 | u8::from(self.overflow) << 5
    }

    /// The nametable VRAM (mirrored view flattened).
    pub fn vram(&self) -> &[u8] {
        &self.vram
    }

    /// The palette RAM (32 bytes).
    pub fn palette_ram(&self) -> &[u8; 0x20] {
        &self.palette
    }

    /// The OAM (256 bytes).
    pub fn oam(&self) -> &[u8; 256] {
        &self.oam
    }

    /// CHR RAM bytes when the cartridge uses RAM (state hashing).
    pub fn chr_ram_bytes(&self) -> Option<&[u8; 0x2000]> {
        self.chr.ram_bytes()
    }

    // ---- output ---------------------------------------------------------------

    /// The palette-index plane of the last completed frame
    /// (256x240 bytes, one palette entry per pixel).
    pub fn last_frame_plane(&self) -> &[u8; 256 * 240] {
        &self.last_plane
    }

    /// A deterministic FNV-1a-64 digest of the last completed frame's
    /// palette-index plane (the frame-level determinism signature).
    pub fn last_plane_hash(&self) -> u64 {
        const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut hash = OFFSET;
        for byte in self.last_plane.iter() {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(PRIME);
        }
        hash
    }

    /// Render the last completed frame to an RGB buffer with the
    /// grayscale and emphasis transforms applied.
    ///
    /// The plane holds palette RAM *indices*; the chain is
    /// index -> the stored 6-bit color -> the 2C02 master palette
    /// (with the emphasis dimming, the documented B1 approximation).
    pub fn last_frame_rgb(&self, out: &mut [u8; 256 * 240 * 3]) {
        let emphasis = self.mask & 0xE0;
        for (i, &index) in self.last_plane.iter().enumerate() {
            let color = self.palette[usize::from(index & 0x1F)];
            let mut rgb = PALETTE_2C02[usize::from(color & 0x3F)];
            if emphasis != 0 {
                let (mut r, mut g, mut b) = (
                    ((rgb >> 16) & 0xFF) as u8,
                    ((rgb >> 8) & 0xFF) as u8,
                    (rgb & 0xFF) as u8,
                );
                let dim = |c: u8, emphasized: bool| -> u8 {
                    if emphasized {
                        c
                    } else {
                        ((u16::from(c) * 3) / 4) as u8
                    }
                };
                r = dim(r, emphasis & 0x40 != 0);
                g = dim(g, emphasis & 0x80 != 0);
                b = dim(b, emphasis & 0x20 != 0);
                rgb = u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b);
            }
            out[i * 3] = ((rgb >> 16) & 0xFF) as u8;
            out[i * 3 + 1] = ((rgb >> 8) & 0xFF) as u8;
            out[i * 3 + 2] = (rgb & 0xFF) as u8;
        }
    }

    /// Automation entry: reset the timing state so that the position
    /// matches the reference trace convention (cycle 7 = dot 21 of
    /// line 0, frame 0) without running the power-on dots.
    pub fn start_automation(&mut self) {
        self.scanline = 0;
        self.dot = 21;
        self.frame = 0;
        self.odd_frame = false;
        self.vbl_set_dot = None;
        self.swallow_set = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ppu_ntsc_chr_ram(mirroring: Mirroring) -> Ppu {
        Ppu::new(Region::Ntsc, Chr::Ram(Box::new([0; 0x2000])), mirroring)
    }

    #[test]
    fn nametable_vertical_mirroring_layout() {
        let mut ppu = ppu_ntsc_chr_ram(Mirroring::Vertical);
        // $2000 and $2800 share table A; $2400 uses table B.
        ppu.vram[0x0000] = 0x11;
        ppu.vram[0x0400] = 0x22;
        assert_eq!(ppu.vram_read(0x2000), 0x11);
        assert_eq!(ppu.vram_read(0x2800), 0x11);
        assert_eq!(ppu.vram_read(0x2400), 0x22);
        assert_eq!(ppu.vram_read(0x2C00), 0x22);
    }

    #[test]
    fn nametable_horizontal_mirroring_layout() {
        let mut ppu = ppu_ntsc_chr_ram(Mirroring::Horizontal);
        ppu.vram[0x0000] = 0x33;
        ppu.vram[0x0400] = 0x44;
        assert_eq!(ppu.vram_read(0x2000), 0x33);
        assert_eq!(ppu.vram_read(0x2400), 0x33);
        assert_eq!(ppu.vram_read(0x2800), 0x44);
        assert_eq!(ppu.vram_read(0x2C00), 0x44);
    }

    #[test]
    fn status_read_clears_vblank_and_w() {
        let mut ppu = ppu_ntsc_chr_ram(Mirroring::Vertical);
        ppu.vblank = true;
        ppu.w = true;
        let value = ppu.read_status(0);
        assert_eq!(value & 0x80, 0x80);
        assert!(!ppu.vblank);
        assert!(!ppu.w);
    }

    #[test]
    fn data_read_is_buffered_except_palette() {
        let mut ppu = ppu_ntsc_chr_ram(Mirroring::Vertical);
        ppu.vram[0x0100] = 0x7E;
        ppu.write_addr(0x21);
        ppu.write_addr(0x00);
        assert_eq!(ppu.read_data(), 0x00, "first read returns the buffer");
        assert_eq!(ppu.read_data(), 0x7E, "second read returns $2100");
        // Palette reads are immediate and honor the mirror rule.
        ppu.write_addr(0x3F);
        ppu.write_addr(0x10);
        ppu.palette_write(0x3F10, 0x2A);
        assert_eq!(ppu.read_data(), 0x2A, "the palette mirror folds");
    }

    #[test]
    fn palette_mirrors_fold_the_sprite_zero_entries() {
        let mut ppu = ppu_ntsc_chr_ram(Mirroring::Vertical);
        ppu.palette_write(0x3F10, 0x21);
        assert_eq!(ppu.palette_read(0x3F00), 0x21);
        ppu.palette_write(0x3F14, 0x22);
        assert_eq!(ppu.palette_read(0x3F04), 0x22);
    }

    #[test]
    fn increment_step_follows_ctrl_bit_two() {
        let mut ppu = ppu_ntsc_chr_ram(Mirroring::Vertical);
        ppu.write_addr(0x20);
        ppu.write_addr(0x00);
        ppu.write_ctrl(0x00);
        ppu.read_data();
        assert_eq!(ppu.scroll_v(), 0x2001);
        ppu.write_ctrl(0x04);
        ppu.read_data();
        assert_eq!(ppu.scroll_v(), 0x2021);
    }

    #[test]
    fn loopy_writes_assemble_t_and_load_v() {
        let mut ppu = ppu_ntsc_chr_ram(Mirroring::Vertical);
        // $2000 selects nametable 1 into t bits 10-11.
        ppu.write_ctrl(0x01);
        assert_eq!(
            ppu.scroll_t() & 0x0C00,
            0x0400,
            "the $2000 nametable select"
        );
        // A $2006 pair owns the full address: it clears the select bits
        // and loads v from t on the second write.
        ppu.write_addr(0x21);
        ppu.write_addr(0x08);
        assert_eq!(ppu.scroll_v(), 0x2108);
        assert_eq!(
            ppu.scroll_t() & 0x0C00,
            0x0000,
            "the $2006 pair owns the address"
        );
        // A $2005 pair after $2002 resets w.
        ppu.read_status(0);
        ppu.write_scroll(0x0A); // fine X = 2, coarse X = 1
        ppu.write_scroll(0x53); // fine Y = 3, coarse Y = 10
        assert_eq!(ppu.scroll_t() & 0x001F, 1);
        assert_eq!((ppu.scroll_t() & 0x03E0) >> 5, 10);
        assert_eq!((ppu.scroll_t() >> 12) & 7, 3);
        assert_eq!(ppu.fine_x(), 2);
    }

    #[test]
    fn vblank_sets_at_line_241_dot_1_and_clears_at_prerender() {
        let mut ppu = ppu_ntsc_chr_ram(Mirroring::Vertical);
        while ppu.position() != (241, 1) {
            ppu.tick();
        }
        assert!(!ppu.vblank, "the set happens while dot 1 ticks");
        ppu.tick();
        assert!(ppu.vblank);
        while ppu.position() != (261, 1) {
            ppu.tick();
        }
        ppu.tick();
        assert!(!ppu.vblank);
        // The frame itself completes at the wrap of the pre-render
        // line's last dot.
        while ppu.frame() < 1 {
            ppu.tick();
        }
        assert_eq!(ppu.frame(), 1);
    }

    #[test]
    fn frame_length_is_341x262_without_rendering() {
        let mut ppu = ppu_ntsc_chr_ram(Mirroring::Vertical);
        let mut dots = 0u64;
        while ppu.frame() < 2 {
            ppu.tick();
            dots += 1;
        }
        assert_eq!(dots, 2 * 341 * 262);
    }

    #[test]
    fn ntsc_odd_frame_skips_a_dot_with_rendering() {
        let mut ppu = ppu_ntsc_chr_ram(Mirroring::Vertical);
        ppu.write_mask(0x08); // background enabled
        let mut dots = 0u64;
        while ppu.frame() < 2 {
            ppu.tick();
            dots += 1;
        }
        // One full frame plus one odd frame missing its last dot.
        assert_eq!(dots, 2 * 341 * 262 - 1);
    }

    #[test]
    fn pal_runs_312_lines_per_frame() {
        let mut ppu = Ppu::new(
            Region::Pal,
            Chr::Ram(Box::new([0; 0x2000])),
            Mirroring::Vertical,
        );
        let mut dots = 0u64;
        while ppu.frame() < 1 {
            ppu.tick();
            dots += 1;
        }
        assert_eq!(dots, 341 * 312);
    }

    #[test]
    fn nmi_edge_follows_ctrl_and_vblank() {
        let mut ppu = ppu_ntsc_chr_ram(Mirroring::Vertical);
        ppu.write_ctrl(0x80);
        while ppu.position() != (241, 1) {
            ppu.tick();
        }
        ppu.tick();
        assert!(ppu.nmi_pending());
        assert!(ppu.take_nmi());
        assert!(!ppu.take_nmi(), "the edge is consumed once");
        // Enabling NMI during vblank re-triggers it.
        ppu.write_ctrl(0x00);
        ppu.write_ctrl(0x80);
        assert!(ppu.take_nmi());
    }

    #[test]
    fn read_status_suppresses_the_nmi_after_the_set() {
        let mut ppu = ppu_ntsc_chr_ram(Mirroring::Vertical);
        ppu.write_ctrl(0x80);
        while ppu.position() != (241, 1) {
            ppu.tick();
        }
        ppu.tick(); // the flag sets and the edge latches.
                    // A read within one CPU cycle after the set returns 1 and
                    // swallows the pending NMI.
        let value = ppu.read_status(0);
        assert_eq!(value & 0x80, 0x80);
        assert!(!ppu.nmi_pending(), "the read suppressed the NMI");
    }

    #[test]
    fn read_status_swallows_an_imminent_set() {
        let mut ppu = ppu_ntsc_chr_ram(Mirroring::Vertical);
        ppu.write_ctrl(0x80);
        // Run to two cycles before the set dot, then read.
        while ppu.position() != (241, 0) {
            ppu.tick();
        }
        ppu.tick(); // position is now (241, 2) — the set dot was 1.
                    // Position (241,2) is three dots past the set-dot... use the
                    // earlier point instead: back up to just before the set.
                    // (The tick above advanced past dot 1; rebuild the state.)
        let mut ppu = ppu_ntsc_chr_ram(Mirroring::Vertical);
        ppu.write_ctrl(0x80);
        while ppu.position() != (241, 0) {
            ppu.tick();
        }
        // Read at the cycle whose dots contain the set dot.
        let value = ppu.read_status(0);
        assert_eq!(value & 0x80, 0x00, "the flag is not set yet");
        // The imminent set must be swallowed: the flag never appears.
        ppu.tick();
        ppu.tick();
        ppu.tick();
        assert!(!ppu.vblank, "the set was swallowed by the read");
        assert!(!ppu.nmi_pending());
    }

    #[test]
    fn sprite_overflow_flag_is_simplified_b1() {
        let mut ppu = ppu_ntsc_chr_ram(Mirroring::Vertical);
        for i in 0..9u8 {
            ppu.oam[usize::from(i) * 4] = 0; // y = 0
        }
        ppu.write_ctrl(0x00);
        ppu.write_mask(0x18);
        while ppu.position() != (0, 65) {
            ppu.tick();
        }
        ppu.tick(); // dot 65 evaluates the sprites of the next line.
        assert!(ppu.status_bits() & 0x20 != 0);
    }

    #[test]
    fn coarse_x_wraps_and_flips_the_nametable_bit() {
        let mut ppu = ppu_ntsc_chr_ram(Mirroring::Vertical);
        ppu.write_ctrl(0x00); // nametable 0
        ppu.write_addr(0x20); // v = $201F (coarse X = 31)
        ppu.write_addr(0x1F);
        ppu.increment_coarse_x();
        assert_eq!(ppu.scroll_v() & 0x0C00, 0x0400, "nametable X flipped");
        assert_eq!(ppu.scroll_v() & 0x001F, 0);
    }

    #[test]
    fn vertical_increment_wraps_at_29_and_flips_nametable_y() {
        let mut ppu = ppu_ntsc_chr_ram(Mirroring::Vertical);
        // v: fine Y = 7, coarse Y = 29, nametable X = 1.
        ppu.v = 0x7000 | (29 << 5) | 0x0400;
        ppu.increment_y();
        assert_eq!(ppu.scroll_v() & 0x7000, 0);
        assert_eq!((ppu.scroll_v() & 0x03E0) >> 5, 0);
        assert_eq!(ppu.scroll_v() & 0x0800, 0x0800, "nametable Y flipped");
        // Coarse Y of 30/31 wraps without the flip.
        ppu.v = 0x7000 | (31 << 5);
        ppu.increment_y();
        assert_eq!(ppu.scroll_v() & 0x0800, 0, "no flip from the 30/31 region");
        assert_eq!((ppu.scroll_v() & 0x03E0) >> 5, 0);
    }

    #[test]
    fn rendering_increment_cadence_moves_v_across_a_line() {
        let mut ppu = ppu_ntsc_chr_ram(Mirroring::Vertical);
        ppu.write_mask(0x08);
        // Start of line 0: v = 0. The line's fetches advance coarse X
        // every 8 dots; at dot 256 the vertical also increments.
        while ppu.position() != (0, 256) {
            ppu.tick();
        }
        // v has been restored? No: h-copy happens at 257. At dot 256
        // (before it ticks), 32 coarse-X increments have occurred:
        // coarse X wrapped to 0 with the nametable bit flipped twice.
        assert_eq!(ppu.scroll_v() & 0x001F, 31, "one increment remains");
        ppu.tick(); // dot 256: the last X increment + fine Y.
        assert_eq!(ppu.scroll_v() & 0x001F, 0);
        assert_eq!(ppu.scroll_v() >> 12, 1, "fine Y advanced");
    }
}
