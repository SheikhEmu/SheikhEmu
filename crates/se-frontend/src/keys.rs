//! Keyboard mapping to the NES buttons (spec §7.3: "a ready default
//! control: arrows + Z/X + Enter + Shift").
//!
//! Physical key codes only: the mapping works on every keyboard
//! layout (an Arabic (or French, or German) layout still gives the
//! player the physical Z/X/Enter/Shift keys, which is what the
//! easy-to-run promise means).
//!
//! - Arrow keys     -> D-pad (Up/Down/Left/Right)
//! - Z              -> A
//! - X              -> B
//! - Enter (Return) -> Start
//! - both Shifts    -> Select

use se_core::{BTN_A, BTN_B, BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SELECT, BTN_START, BTN_UP};
use winit::keyboard::KeyCode;

/// The button bit a physical key maps to (None: not a game key).
pub fn key_to_bit(code: KeyCode) -> Option<u8> {
    let bit = match code {
        KeyCode::ArrowUp => BTN_UP,
        KeyCode::ArrowDown => BTN_DOWN,
        KeyCode::ArrowLeft => BTN_LEFT,
        KeyCode::ArrowRight => BTN_RIGHT,
        KeyCode::KeyZ => BTN_A,
        KeyCode::KeyX => BTN_B,
        KeyCode::Enter | KeyCode::NumpadEnter => BTN_START,
        KeyCode::ShiftLeft | KeyCode::ShiftRight => BTN_SELECT,
        _ => return None,
    };
    Some(bit)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_map_the_dpad() {
        assert_eq!(key_to_bit(KeyCode::ArrowUp), Some(BTN_UP));
        assert_eq!(key_to_bit(KeyCode::ArrowDown), Some(BTN_DOWN));
        assert_eq!(key_to_bit(KeyCode::ArrowLeft), Some(BTN_LEFT));
        assert_eq!(key_to_bit(KeyCode::ArrowRight), Some(BTN_RIGHT));
    }

    #[test]
    fn zx_are_a_b() {
        assert_eq!(key_to_bit(KeyCode::KeyZ), Some(BTN_A));
        assert_eq!(key_to_bit(KeyCode::KeyX), Some(BTN_B));
    }

    #[test]
    fn enter_is_start_and_both_shifts_select() {
        assert_eq!(key_to_bit(KeyCode::Enter), Some(BTN_START));
        assert_eq!(key_to_bit(KeyCode::NumpadEnter), Some(BTN_START));
        assert_eq!(key_to_bit(KeyCode::ShiftLeft), Some(BTN_SELECT));
        assert_eq!(key_to_bit(KeyCode::ShiftRight), Some(BTN_SELECT));
    }

    #[test]
    fn unrelated_keys_map_to_none() {
        assert_eq!(key_to_bit(KeyCode::KeyQ), None);
        assert_eq!(key_to_bit(KeyCode::Space), None);
        assert_eq!(key_to_bit(KeyCode::Escape), None);
    }

    #[test]
    fn each_mapped_key_is_a_distinct_single_bit() {
        for key in [
            KeyCode::ArrowUp,
            KeyCode::ArrowDown,
            KeyCode::ArrowLeft,
            KeyCode::ArrowRight,
            KeyCode::KeyZ,
            KeyCode::KeyX,
            KeyCode::Enter,
            KeyCode::ShiftLeft,
        ] {
            let bit = key_to_bit(key).expect("mapped");
            assert_eq!(bit.count_ones(), 1, "one button per key");
        }
    }
}
