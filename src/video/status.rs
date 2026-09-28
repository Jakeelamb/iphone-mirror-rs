//! Small fixed bitmap alphabet for transient controls, without a font dependency.
const CAPACITY: usize = 80;

#[derive(PartialEq, Eq)]
pub(super) struct StatusText {
    // First vec4 holds the character count, followed by one vec4 per 5x7 glyph.
    words: [[u32; 4]; CAPACITY + 1],
}

impl Default for StatusText {
    fn default() -> Self {
        Self {
            words: [[0; 4]; CAPACITY + 1],
        }
    }
}

impl StatusText {
    pub const BYTE_SIZE: u64 = ((CAPACITY + 1) * 16) as u64;

    pub fn set(&mut self, text: &str) -> bool {
        let mut next = Self::default();
        for (index, character) in text.chars().take(CAPACITY).enumerate() {
            let columns = glyph(character.to_ascii_uppercase());
            let bits = columns
                .iter()
                .enumerate()
                .fold(0_u64, |bits, (column, value)| {
                    bits | (u64::from(*value) << (column * 7))
                });
            next.words[index + 1] = [bits as u32, (bits >> 32) as u32, 0, 0];
            next.words[0][0] += 1;
        }
        let changed = *self != next;
        *self = next;
        changed
    }

    pub fn bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.words)
    }
}

// Each column uses seven low bits, top to bottom. Patterns are embedded data,
// not a rasterized external font or a runtime atlas allocation.
fn glyph(character: char) -> [u8; 5] {
    match character {
        'A' => [0x7e, 0x11, 0x11, 0x11, 0x7e],
        'B' => [0x7f, 0x49, 0x49, 0x49, 0x36],
        'C' => [0x3e, 0x41, 0x41, 0x41, 0x22],
        'D' => [0x7f, 0x41, 0x41, 0x22, 0x1c],
        'E' => [0x7f, 0x49, 0x49, 0x49, 0x41],
        'F' => [0x7f, 0x09, 0x09, 0x09, 0x01],
        'G' => [0x3e, 0x41, 0x49, 0x49, 0x7a],
        'H' => [0x7f, 0x08, 0x08, 0x08, 0x7f],
        'I' => [0, 0x41, 0x7f, 0x41, 0],
        'J' => [0x20, 0x40, 0x41, 0x3f, 0x01],
        'K' => [0x7f, 0x08, 0x14, 0x22, 0x41],
        'L' => [0x7f, 0x40, 0x40, 0x40, 0x40],
        'M' => [0x7f, 0x02, 0x0c, 0x02, 0x7f],
        'N' => [0x7f, 0x04, 0x08, 0x10, 0x7f],
        'O' => [0x3e, 0x41, 0x41, 0x41, 0x3e],
        'P' => [0x7f, 0x09, 0x09, 0x09, 0x06],
        'Q' => [0x3e, 0x41, 0x51, 0x21, 0x5e],
        'R' => [0x7f, 0x09, 0x19, 0x29, 0x46],
        'S' => [0x46, 0x49, 0x49, 0x49, 0x31],
        'T' => [0x01, 0x01, 0x7f, 0x01, 0x01],
        'U' => [0x3f, 0x40, 0x40, 0x40, 0x3f],
        'V' => [0x1f, 0x20, 0x40, 0x20, 0x1f],
        'W' => [0x3f, 0x40, 0x38, 0x40, 0x3f],
        'X' => [0x63, 0x14, 0x08, 0x14, 0x63],
        'Y' => [0x03, 0x04, 0x78, 0x04, 0x03],
        'Z' => [0x61, 0x51, 0x49, 0x45, 0x43],
        '0' => [0x3e, 0x51, 0x49, 0x45, 0x3e],
        '1' => [0, 0x42, 0x7f, 0x40, 0],
        '2' => [0x42, 0x61, 0x51, 0x49, 0x46],
        '3' => [0x21, 0x41, 0x45, 0x4b, 0x31],
        '4' => [0x18, 0x14, 0x12, 0x7f, 0x10],
        '5' => [0x27, 0x45, 0x45, 0x45, 0x39],
        '6' => [0x3c, 0x4a, 0x49, 0x49, 0x30],
        '7' => [0x01, 0x71, 0x09, 0x05, 0x03],
        '8' => [0x36, 0x49, 0x49, 0x49, 0x36],
        '9' => [0x06, 0x49, 0x49, 0x29, 0x1e],
        ' ' => [0; 5],
        ':' => [0, 0x36, 0x36, 0, 0],
        '.' => [0, 0x60, 0x60, 0, 0],
        ',' => [0, 0x40, 0x30, 0, 0],
        '-' | '–' | '—' => [0x08; 5],
        '_' => [0x40; 5],
        '+' => [0x08, 0x08, 0x3e, 0x08, 0x08],
        '=' => [0x14; 5],
        '/' => [0x20, 0x10, 0x08, 0x04, 0x02],
        '(' => [0, 0x1c, 0x22, 0x41, 0],
        ')' => [0, 0x41, 0x22, 0x1c, 0],
        '[' => [0, 0x7f, 0x41, 0x41, 0],
        ']' => [0, 0x41, 0x41, 0x7f, 0],
        '!' => [0, 0, 0x5f, 0, 0],
        '%' => [0x63, 0x13, 0x08, 0x64, 0x63],
        _ => [0x02, 0x01, 0x51, 0x09, 0x06],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_bounded_case_normalized_and_cleared() {
        let mut status = StatusText::default();
        assert!(!status.set(""));
        assert!(status.set("F9: click MOVE"));
        assert!(!status.set("F9: CLICK MOVE"));
        assert_eq!(status.words[0][0], 14);
        assert!(status.set(&"A".repeat(100)));
        assert_eq!(status.words[0][0], 80);
        assert_eq!(status.bytes().len() as u64, StatusText::BYTE_SIZE);
        assert!(status.set(""));
        assert!(status.words.iter().all(|word| *word == [0; 4]));
    }

    #[test]
    fn packed_columns_preserve_all_35_pixels() {
        let mut status = StatusText::default();
        status.set("AZ09");
        for (index, character) in "AZ09".chars().enumerate() {
            let word = status.words[index + 1];
            let bits = u64::from(word[0]) | (u64::from(word[1]) << 32);
            for (column, expected) in glyph(character).iter().enumerate() {
                assert_eq!(((bits >> (column * 7)) & 127) as u8, *expected);
            }
        }
    }
}
