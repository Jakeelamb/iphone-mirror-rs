//! Allocation-free HID encoding and input state, independent of the window system.
//!
//! Report offsets follow the reference project's UniversalControl captures.
//! An active DisplayService stream must exist before these reports are accepted.
use std::collections::VecDeque;

pub const TOUCHSCREEN_SERVICE_ID: u64 = 257;
pub const KEYBOARD_SERVICE_ID: u64 = 0x1_0000_2001;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    Begin,
    Move,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HidEvent {
    Touch { phase: TouchPhase, report: [u8; 58] },
    Keyboard([u8; 39]),
    Home { pressed: bool },
}

pub fn touchscreen_report(pressed: bool, x: u16, y: u16, timestamp: u64) -> [u8; 58] {
    let mut report = [0; 58];
    report[..4].copy_from_slice(&[9, 1, 5, if pressed { 0xc2 } else { 2 }]);
    report[4..6].copy_from_slice(&x.to_le_bytes());
    report[6..8].copy_from_slice(&y.to_le_bytes());
    report[40] = 2;
    report[44..50].copy_from_slice(&timestamp.to_le_bytes()[..6]);
    report
}

pub fn keyboard_report(bitmap: &[u8; 30], timestamp: u64) -> [u8; 39] {
    let mut report = [0; 39];
    report[0] = 1;
    report[1..31].copy_from_slice(bitmap);
    report[31..37].copy_from_slice(&timestamp.to_le_bytes()[..6]);
    report
}

/// Convert normalized displayed coordinates to the encoded buffer's coordinates.
/// `rotation` is clockwise display rotation in degrees. Nonfinite values are rejected.
pub fn normalized_position(x: f64, y: f64, rotation: u16) -> Option<(u16, u16)> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let (x, y) = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
    let (x, y) = match rotation % 360 {
        90 => (y, 1.0 - x),
        180 => (1.0 - x, 1.0 - y),
        270 => (1.0 - y, x),
        _ => (x, y),
    };
    Some(((x * 65535.0).round() as u16, (y * 65535.0).round() as u16))
}

/// US ASCII mapping: HID keyboard usage and whether Shift is needed.
pub fn ascii_usage(c: char) -> Option<(u8, bool)> {
    Some(match c {
        'a'..='z' => (4 + (c as u8 - b'a'), false),
        'A'..='Z' => (4 + (c as u8 - b'A'), true),
        '1'..='9' => (30 + (c as u8 - b'1'), false),
        '0' => (39, false),
        '\n' | '\r' => (40, false),
        '\u{8}' => (42, false),
        '\t' => (43, false),
        ' ' => (44, false),
        '-' => (45, false),
        '=' => (46, false),
        '[' => (47, false),
        ']' => (48, false),
        '\\' => (49, false),
        ';' => (51, false),
        '\'' => (52, false),
        '`' => (53, false),
        ',' => (54, false),
        '.' => (55, false),
        '/' => (56, false),
        '!' => (30, true),
        '@' => (31, true),
        '#' => (32, true),
        '$' => (33, true),
        '%' => (34, true),
        '^' => (35, true),
        '&' => (36, true),
        '*' => (37, true),
        '(' => (38, true),
        ')' => (39, true),
        '_' => (45, true),
        '+' => (46, true),
        '{' => (47, true),
        '}' => (48, true),
        '|' => (49, true),
        ':' => (51, true),
        '"' => (52, true),
        '~' => (53, true),
        '<' => (54, true),
        '>' => (55, true),
        '?' => (56, true),
        _ => return None,
    })
}

#[derive(Debug, Default)]
pub struct InputState {
    contact: Option<(u16, u16)>,
    keys: [u8; 30],
    home: bool,
}

impl InputState {
    pub fn touch_begin(&mut self, x: u16, y: u16, timestamp: u64) -> Option<HidEvent> {
        if self.contact.is_some() {
            return None;
        }
        self.contact = Some((x, y));
        Some(HidEvent::Touch {
            phase: TouchPhase::Begin,
            report: touchscreen_report(true, x, y, timestamp),
        })
    }

    pub fn touch_move(&mut self, x: u16, y: u16, timestamp: u64) -> Option<HidEvent> {
        self.contact?;
        if self.contact == Some((x, y)) {
            return None;
        }
        self.contact = Some((x, y));
        Some(HidEvent::Touch {
            phase: TouchPhase::Move,
            report: touchscreen_report(true, x, y, timestamp),
        })
    }

    pub fn touch_end(&mut self, timestamp: u64) -> Option<HidEvent> {
        let (x, y) = self.contact.take()?;
        Some(HidEvent::Touch {
            phase: TouchPhase::End,
            report: touchscreen_report(false, x, y, timestamp),
        })
    }

    pub fn set_key(&mut self, usage: u8, pressed: bool, timestamp: u64) -> Option<HidEvent> {
        if usage >= 240 {
            return None;
        }
        let (index, mask) = ((usage / 8) as usize, 1 << (usage % 8));
        if (self.keys[index] & mask != 0) == pressed {
            return None;
        }
        if pressed {
            self.keys[index] |= mask;
        } else {
            self.keys[index] &= !mask;
        }
        Some(HidEvent::Keyboard(keyboard_report(&self.keys, timestamp)))
    }

    pub fn home(&mut self, pressed: bool) -> Option<HidEvent> {
        if self.home == pressed {
            return None;
        }
        self.home = pressed;
        Some(HidEvent::Home { pressed })
    }

    /// Call on focus loss and before shutdown; drain every returned event.
    pub fn release_all(&mut self, timestamp: u64) -> [Option<HidEvent>; 3] {
        let touch = self.touch_end(timestamp);
        let keyboard = if self.keys.iter().any(|&byte| byte != 0) {
            self.keys.fill(0);
            Some(HidEvent::Keyboard(keyboard_report(&self.keys, timestamp)))
        } else {
            None
        };
        [touch, keyboard, self.home(false)]
    }
}

/// A bounded queue; only adjacent motion samples may be replaced. A full queue
/// returns ownership of transitions to the caller: retry with backpressure, never
/// discard an error. This preserves releases without an unbounded event backlog.
#[derive(Debug)]
pub struct InputQueue {
    events: VecDeque<HidEvent>,
    limit: usize,
}

impl InputQueue {
    pub fn new(capacity: usize) -> Self {
        Self {
            events: VecDeque::with_capacity(capacity),
            limit: capacity,
        }
    }
    pub fn try_push(&mut self, event: HidEvent) -> Result<(), HidEvent> {
        if matches!(
            event,
            HidEvent::Touch {
                phase: TouchPhase::Move,
                ..
            }
        ) && let Some(
            last @ HidEvent::Touch {
                phase: TouchPhase::Move,
                ..
            },
        ) = self.events.back_mut()
        {
            *last = event;
            return Ok(());
        }
        if self.events.len() == self.limit {
            return Err(event);
        }
        self.events.push_back(event);
        Ok(())
    }
    pub fn pop(&mut self) -> Option<HidEvent> {
        self.events.pop_front()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TouchSample {
    pub phase: TouchPhase,
    pub x: u16,
    pub y: u16,
}

/// Nine touch samples spaced 15ms apart followed by release. Positive wheel
/// lines move the finger down; edge clamping avoids iOS system gestures.
pub fn wheel_gesture(x: f64, y: f64, lines: f64, rotation: u16) -> Option<[TouchSample; 10]> {
    if !x.is_finite() || !y.is_finite() || !lines.is_finite() || lines == 0.0 {
        return None;
    }
    let (x, y) = (x.clamp(0.05, 0.95), y.clamp(0.2, 0.8));
    let end = (y + lines.clamp(-7.0, 7.0) * 0.1).clamp(0.1, 0.9);
    let mut samples = [TouchSample {
        phase: TouchPhase::Move,
        x: 0,
        y: 0,
    }; 10];
    for (i, sample) in samples.iter_mut().enumerate() {
        let (px, py) = normalized_position(x, y + (end - y) * i.min(8) as f64 / 8.0, rotation)?;
        *sample = TouchSample {
            phase: if i == 0 {
                TouchPhase::Begin
            } else if i == 9 {
                TouchPhase::End
            } else {
                TouchPhase::Move
            },
            x: px,
            y: py,
        };
    }
    Some(samples)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    #[test]
    fn wire_report_matches_capture_offsets() {
        let r = touchscreen_report(true, 0x1234, 0xabcd, 0xffff_1122_3344_5566);
        assert_eq!(&r[..8], &[9, 1, 5, 0xc2, 0x34, 0x12, 0xcd, 0xab]);
        assert!(r[8..40].iter().all(|&v| v == 0));
        assert_eq!(
            &r[40..50],
            &[2, 0, 0, 0, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11]
        );
        assert_eq!(touchscreen_report(false, 0, 0, 0)[3], 2);
    }
    #[test]
    fn keyboard_bitmap_and_focus_release() {
        let mut s = InputState::default();
        s.touch_begin(10, 20, 0);
        s.home(true);
        s.set_key(4, true, 0);
        let Some(HidEvent::Keyboard(report)) = s.set_key(225, true, 7) else {
            panic!("keyboard report");
        };
        assert_eq!(report[1], 16);
        assert_eq!(report[29], 2);
        assert_eq!(report[31], 7);
        assert!(s.set_key(225, true, 7).is_none());
        assert!(s.release_all(8).into_iter().all(|e| e.is_some()));
        assert_eq!(s.release_all(9), [None; 3]);
        assert!(s.touch_move(50, 60, 10).is_none());
    }
    #[test]
    fn rotation_and_coordinates_are_bounded() {
        assert_eq!(normalized_position(0.0, 1.0, 90), Some((65535, 65535)));
        assert_eq!(normalized_position(0.0, 1.0, 270), Some((0, 0)));
        assert_eq!(normalized_position(-5.0, 10.0, 0), Some((0, 65535)));
        assert_eq!(normalized_position(f64::NAN, 0.0, 0), None);
        for r in [0, 90, 180, 270] {
            for x in 0..=20 {
                for y in 0..=20 {
                    let (a, b) = normalized_position(x as f64 / 20.0, y as f64 / 20.0, r).unwrap();
                    let (c, d) = normalized_position(
                        a as f64 / 65535.0,
                        b as f64 / 65535.0,
                        (360 - r) % 360,
                    )
                    .unwrap();
                    assert!((c as i32 - (x as f64 * 65535.0 / 20.0).round() as i32).abs() <= 1);
                    assert!((d as i32 - (y as f64 * 65535.0 / 20.0).round() as i32).abs() <= 1);
                }
            }
        }
    }
    #[test]
    fn move_coalescing_never_overwrites_transitions() {
        let mut q = InputQueue::new(3);
        let mut s = InputState::default();
        let down = s.touch_begin(1, 1, 0).unwrap();
        q.try_push(down).unwrap();
        q.try_push(s.touch_move(2, 2, 1).unwrap()).unwrap();
        let latest = s.touch_move(3, 3, 2).unwrap();
        q.try_push(latest).unwrap();
        let up = s.touch_end(3).unwrap();
        q.try_push(up).unwrap();
        let next = s.touch_begin(4, 4, 4).unwrap();
        assert_eq!(q.try_push(next), Err(next));
        assert_eq!(q.pop(), Some(down));
        assert_eq!(q.pop(), Some(latest));
        assert_eq!(q.pop(), Some(up));
    }
    #[test]
    fn ascii_and_wheel() {
        for c in ' '..='~' {
            assert!(ascii_usage(c).is_some(), "{c}");
        }
        assert_eq!(ascii_usage('é'), None);
        assert_eq!(ascii_usage('A'), Some((4, true)));
        let samples = wheel_gesture(0.5, 0.5, -1.0, 0).unwrap();
        assert_eq!(samples[0].phase, TouchPhase::Begin);
        assert_eq!(samples[9].phase, TouchPhase::End);
        assert!(samples.windows(2).all(|s| s[1].y <= s[0].y));
        assert_eq!((samples[8].x, samples[8].y), (samples[9].x, samples[9].y));
    }

    #[test]
    fn landscape_taps_target_the_visible_quadrant_in_encoded_space() {
        // A point toward the displayed upper-left must address different
        // encoded quadrants for the two landscape orientations.
        assert_eq!(normalized_position(0.25, 0.25, 90), Some((16384, 49151)));
        assert_eq!(normalized_position(0.25, 0.25, 270), Some((49151, 16384)));
        assert_eq!(normalized_position(0.75, 0.25, 90), Some((16384, 16384)));
        assert_eq!(normalized_position(0.75, 0.25, 270), Some((49151, 49151)));
    }

    #[test]
    fn wheel_follows_display_vertical_in_both_landscape_orientations() {
        for rotation in [0, 90, 180, 270] {
            for lines in [-2.0, 2.0] {
                let samples = wheel_gesture(0.4, 0.5, lines, rotation).unwrap();
                assert_eq!(samples[0].phase, TouchPhase::Begin);
                assert_eq!(samples[9].phase, TouchPhase::End);
                for pair in samples[..9].windows(2) {
                    let dx = i32::from(pair[1].x) - i32::from(pair[0].x);
                    let dy = i32::from(pair[1].y) - i32::from(pair[0].y);
                    let sign = if lines > 0.0 { 1 } else { -1 };
                    match rotation {
                        0 => {
                            assert_eq!(dx, 0);
                            assert_eq!(dy.signum(), sign);
                        }
                        90 => {
                            assert_eq!(dy, 0);
                            assert_eq!(dx.signum(), sign);
                        }
                        180 => {
                            assert_eq!(dx, 0);
                            assert_eq!(dy.signum(), -sign);
                        }
                        270 => {
                            assert_eq!(dy, 0);
                            assert_eq!(dx.signum(), -sign);
                        }
                        _ => unreachable!(),
                    }
                }
                // Lifting never teleports the contact at either end of a swipe.
                assert_eq!((samples[8].x, samples[8].y), (samples[9].x, samples[9].y));
            }
        }
    }
}
