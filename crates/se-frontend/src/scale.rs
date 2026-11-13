//! Integer scaling and letterboxing for the 256x240 NES picture
//! (spec §5.5: "correct (integer) upscaling").
//!
//! The picture is never stretched to a fractional factor: the largest
//! integer factor that fits the viewport is used and the remainder is
//! split evenly into side/top letterbox bars (painted black).

/// The NES picture dimensions.
pub const SCREEN_W: u32 = 256;
pub const SCREEN_H: u32 = 240;

/// Where the scaled picture goes inside a `width` x `height` viewport.
///
/// `scale` is at least 1 (a viewport smaller than the picture simply
/// clips; a window that small is the user's own doing, the frontend
/// never opens one that small).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub scale: u32,
    pub offset_x: u32,
    pub offset_y: u32,
}

/// Compute the placement of the 256x240 picture in a viewport.
pub fn placement(width: u32, height: u32) -> Placement {
    let by_width = if width >= SCREEN_W {
        width / SCREEN_W
    } else {
        0
    };
    let by_height = if height >= SCREEN_H {
        height / SCREEN_H
    } else {
        0
    };
    let scale = by_width.min(by_height).max(1);
    Placement {
        scale,
        offset_x: width.saturating_sub(SCREEN_W * scale) / 2,
        offset_y: height.saturating_sub(SCREEN_H * scale) / 2,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_multiple_gives_pure_integer_scale() {
        // 768x720 = 3x: no letterbox at all.
        assert_eq!(
            placement(768, 720),
            Placement {
                scale: 3,
                offset_x: 0,
                offset_y: 0
            }
        );
    }

    #[test]
    fn height_limits_the_factor_and_bars_are_centered() {
        // 800x600: 800/256 = 3 but 600/240 = 2 => scale 2,
        // 800 - 512 = 288 -> 144 px each side, 600 - 480 = 120 -> 60 top/bottom.
        assert_eq!(
            placement(800, 600),
            Placement {
                scale: 2,
                offset_x: 144,
                offset_y: 60
            }
        );
    }

    #[test]
    fn sub_picture_viewport_keeps_scale_one_no_wraparound() {
        // The saturating subtraction must not underflow.
        let p = placement(100, 90);
        assert_eq!(p.scale, 1);
        assert_eq!(p.offset_x, 0);
        assert_eq!(p.offset_y, 0);
    }

    #[test]
    fn wide_viewport_letterboxes_horizontally_only() {
        // 1024x240: scale 1 (height-limited), 768 -> 384 px each side.
        let p = placement(1024, 240);
        assert_eq!(p.scale, 1);
        assert_eq!(p.offset_x, 384);
        assert_eq!(p.offset_y, 0);
    }

    #[test]
    fn one_pixel_viewport_does_not_panic() {
        let p = placement(1, 1);
        assert_eq!(p.scale, 1);
        assert_eq!(p.offset_x, 0);
        assert_eq!(p.offset_y, 0);
    }
}
