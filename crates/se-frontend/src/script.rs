//! A deterministic input scenario for the frontend's replay mode (D-32).
//!
//! The same text format and the same frame-boundary contract as the CLI's
//! `--input-script` (M3.1): a plain-text schedule of button snapshots
//! applied to controller 1 at frame boundaries. The frame index counts
//! completed frames; entry N applies from the start of frame N, and the
//! last snapshot stays held until a later entry replaces it (the CLI's
//! sticky `buttons` variable — a released-key line must say so explicitly
//! with a bare frame number).
//!
//! The replay mode exists for evidence (D-32): the owner's display
//! standard is a pixel comparison between a known-state window screenshot
//! and the same-state CLI frame, and both sides must consume identical
//! inputs at identical frame boundaries to be at the same state.

use se_core::{BTN_A, BTN_B, BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SELECT, BTN_START, BTN_UP};

/// A parsed scenario: `(frame, buttons)` snapshots, sorted and deduplicated.
#[derive(Debug)]
pub struct InputScript {
    schedule: Vec<(u32, u8)>,
}

impl InputScript {
    /// Parse the scenario text. Plain matching with Arabic errors
    /// (D-06), mirroring the CLI parser line for line.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut schedule: Vec<(u32, u8)> = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let frame_text = parts.next().unwrap_or_default();
            let frame = frame_text
                .parse::<u32>()
                .map_err(|_| format!("السطر {}: رقم إطار غير صالح: {frame_text}", index + 1))?;
            let mut buttons = 0u8;
            for name in parts {
                buttons |= match name.to_ascii_lowercase().as_str() {
                    "a" => BTN_A,
                    "b" => BTN_B,
                    "select" => BTN_SELECT,
                    "start" => BTN_START,
                    "up" => BTN_UP,
                    "down" => BTN_DOWN,
                    "left" => BTN_LEFT,
                    "right" => BTN_RIGHT,
                    other => {
                        return Err(format!("السطر {}: اسم زر غير معروف: {other}", index + 1));
                    }
                };
            }
            schedule.push((frame, buttons));
        }
        // Frame order tolerance: sort, then let the last entry at a
        // duplicated frame supersede earlier ones.
        schedule.sort_by_key(|(frame, _)| *frame);
        let mut merged: Vec<(u32, u8)> = Vec::new();
        for (frame, buttons) in schedule {
            if let Some(last) = merged.last_mut() {
                if last.0 == frame {
                    last.1 = buttons;
                    continue;
                }
            }
            merged.push((frame, buttons));
        }
        Ok(InputScript { schedule: merged })
    }

    /// The buttons held during frame `frame`. `cursor` caches the parse
    /// position across calls (frames are asked in order); the returned
    /// snapshot is sticky: it stays held until a later entry replaces it.
    pub fn buttons_at(&self, frame: u32, cursor: &mut usize) -> u8 {
        while *cursor < self.schedule.len() && self.schedule[*cursor].0 <= frame {
            *cursor += 1;
        }
        if *cursor == 0 {
            0
        } else {
            self.schedule[*cursor - 1].1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::InputScript;

    #[test]
    fn parses_names_comments_and_blank_lines() {
        let script =
            InputScript::parse("# scenario\n\n30 start\n32\n200 left\n260 left\n262\n").unwrap();
        assert_eq!(
            script.schedule,
            vec![(30, 0x08), (32, 0), (200, 0x40), (260, 0x40), (262, 0)]
        );
    }

    #[test]
    fn rejects_unknown_buttons_with_line_number() {
        let error = InputScript::parse("5 turbo").unwrap_err();
        assert!(error.contains("السطر 1"), "actual: {error}");
        assert!(error.contains("turbo"), "actual: {error}");
    }

    #[test]
    fn rejects_bad_frame_numbers_with_line_number() {
        let error = InputScript::parse("soon start").unwrap_err();
        assert!(error.contains("السطر 1"), "actual: {error}");
    }

    #[test]
    fn last_entry_at_a_duplicated_frame_wins() {
        let script = InputScript::parse("30 start\n30 left\n31\n").unwrap();
        assert_eq!(script.schedule, vec![(30, 0x40), (31, 0)]);
    }

    #[test]
    fn buttons_are_sticky_between_entries() {
        let script = InputScript::parse("30 start\n60 start\n90\n").unwrap();
        let mut cursor = 0usize;
        assert_eq!(script.buttons_at(0, &mut cursor), 0);
        assert_eq!(script.buttons_at(29, &mut cursor), 0);
        assert_eq!(script.buttons_at(30, &mut cursor), 0x08);
        assert_eq!(script.buttons_at(59, &mut cursor), 0x08);
        assert_eq!(script.buttons_at(60, &mut cursor), 0x08);
        assert_eq!(script.buttons_at(89, &mut cursor), 0x08);
        // The bare `90` releases everything from frame 90 on.
        assert_eq!(script.buttons_at(90, &mut cursor), 0);
        assert_eq!(script.buttons_at(2100, &mut cursor), 0);
    }

    #[test]
    fn cursor_is_only_advised_and_requeries_safely() {
        // A fresh cursor anywhere in the schedule still answers correctly
        // (the entries before it are skipped, the sticky state is the last
        // one at or before the frame).
        let script = InputScript::parse("30 start\n200 left\n").unwrap();
        let mut cursor = 0usize;
        let _ = script.buttons_at(500, &mut cursor);
        assert_eq!(script.buttons_at(500, &mut cursor), 0x40);
    }

    #[test]
    fn empty_script_holds_nothing() {
        let script = InputScript::parse("# nothing\n\n").unwrap();
        let mut cursor = 0usize;
        assert_eq!(script.buttons_at(0, &mut cursor), 0);
        assert_eq!(script.buttons_at(9999, &mut cursor), 0);
    }
}
