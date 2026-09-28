//! Clockwise visual turns on top of the encoded buffer, matching SpringBoard.

pub(super) fn normalized_rotation(degrees: u16) -> u16 {
    match degrees % 360 {
        90 => 90,
        180 => 180,
        270 => 270,
        _ => 0,
    }
}

/// iOS can either keep portrait-encoded video or re-encode a landscape buffer.
/// Suppress quarter-turns in the second case so it is not rotated twice.
pub fn visual_rotation(interface_orientation: u8, buffer_width: u32, buffer_height: u32) -> u16 {
    let wanted = match interface_orientation {
        2 => 180,
        3 => 270,
        4 => 90,
        _ => 0,
    };
    if matches!(wanted, 90 | 270) && buffer_width > buffer_height && buffer_height > 0 {
        0
    } else {
        wanted
    }
}

/// Dimensions after the visual turn; the original decoded planes stay intact.
pub fn displayed_size(width: u32, height: u32, rotation: u16) -> (u32, u32) {
    if matches!(normalized_rotation(rotation), 90 | 270) {
        (height, width)
    } else {
        (width, height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn springboard_orientations_match_reference_visual_turns() {
        for (orientation, expected) in [(0, 0), (1, 0), (2, 180), (3, 270), (4, 90), (255, 0)] {
            assert_eq!(visual_rotation(orientation, 1184, 2576), expected);
            assert_eq!(visual_rotation(orientation, 0, 0), expected);
        }
    }

    #[test]
    fn landscape_encoded_buffers_are_not_rotated_twice() {
        for orientation in [3, 4] {
            assert_eq!(visual_rotation(orientation, 2576, 1184), 0);
        }
        assert_eq!(visual_rotation(2, 2576, 1184), 180);
        assert_eq!(visual_rotation(3, 100, 0), 270);
        assert_eq!(visual_rotation(4, 100, 100), 90);
    }

    #[test]
    fn display_dimensions_swap_only_for_quarter_turns() {
        for degrees in [0, 180, 360, 540, 45] {
            assert_eq!(displayed_size(1184, 2576, degrees), (1184, 2576));
        }
        for degrees in [90, 270, 450, 630] {
            assert_eq!(displayed_size(1184, 2576, degrees), (2576, 1184));
        }
    }
}
