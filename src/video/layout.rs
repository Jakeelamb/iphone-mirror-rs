//! Physical-pixel geometry shared by GPU drawing and pointer hit testing.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn contains(self, x: f64, y: f64) -> bool {
        x.is_finite()
            && y.is_finite()
            && self.width > 0.0
            && self.height > 0.0
            && x >= self.x
            && y >= self.y
            && x <= self.x + self.width
            && y <= self.y + self.height
    }

    fn contains_rounded(self, x: f64, y: f64, radius: f64) -> bool {
        if !self.contains(x, y) {
            return false;
        }
        let (cx, cy) = self.corner_center(x, y, radius);
        (x - cx).hypot(y - cy) <= radius
    }

    fn corner_center(self, x: f64, y: f64, radius: f64) -> (f64, f64) {
        (
            x.clamp(self.x + radius, self.x + self.width - radius),
            y.clamp(self.y + radius, self.y + self.height - radius),
        )
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ViewerLayout {
    pub screen: Rect,
    pub home: Rect,
    pub footer: Rect,
    pub corner_radius: f64,
}

impl ViewerLayout {
    pub fn new(
        window_width: u32,
        window_height: u32,
        frame_width: u32,
        frame_height: u32,
        scale_factor: f64,
    ) -> Self {
        let dpi = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        let width = f64::from(window_width);
        let height = f64::from(window_height);
        let footer_height = (48.0 * dpi).min(height);
        let video_height = height - footer_height;
        let footer = Rect {
            x: 0.0,
            y: video_height,
            width,
            height: footer_height,
        };
        let button_width = (80.0 * dpi).min((width - 16.0 * dpi).max(0.0));
        let button_height = (32.0 * dpi).min((footer_height - 8.0 * dpi).max(0.0));
        let home = Rect {
            x: (width - button_width) / 2.0,
            y: video_height + (footer_height - button_height) / 2.0,
            width: button_width,
            height: button_height,
        };
        if frame_width == 0 || frame_height == 0 || width == 0.0 || video_height == 0.0 {
            return Self {
                screen: Rect::default(),
                home,
                footer,
                corner_radius: 0.0,
            };
        }
        let scale = (width / f64::from(frame_width)).min(video_height / f64::from(frame_height));
        let screen_width = (f64::from(frame_width) * scale).min(width);
        let screen_height = (f64::from(frame_height) * scale).min(video_height);
        Self {
            screen: Rect {
                x: (width - screen_width) / 2.0,
                y: (video_height - screen_height) / 2.0,
                width: screen_width,
                height: screen_height,
            },
            home,
            footer,
            corner_radius: screen_width.min(screen_height) * 0.12,
        }
    }

    pub fn home_contains(self, x: f64, y: f64) -> bool {
        self.home
            .contains_rounded(x, y, self.home.width.min(self.home.height) / 2.0)
    }

    /// New touches exclude clipped corners and the footer. Existing drags may
    /// request clamping, which projects outside points onto the rounded boundary.
    pub fn screen_position(self, x: f64, y: f64, clamp: bool) -> Option<(f64, f64)> {
        if !x.is_finite() || !y.is_finite() || self.screen.width <= 0.0 || self.screen.height <= 0.0
        {
            return None;
        }
        let (x, y) = if clamp {
            let mut x = x.clamp(self.screen.x, self.screen.x + self.screen.width);
            let mut y = y.clamp(self.screen.y, self.screen.y + self.screen.height);
            let (cx, cy) = self.screen.corner_center(x, y, self.corner_radius);
            let distance = (x - cx).hypot(y - cy);
            if distance > self.corner_radius {
                x = cx + (x - cx) * self.corner_radius / distance;
                y = cy + (y - cy) * self.corner_radius / distance;
            }
            (x, y)
        } else {
            if !self.screen.contains_rounded(x, y, self.corner_radius) {
                return None;
            }
            (x, y)
        };
        Some((
            (x - self.screen.x) / self.screen.width,
            (y - self.screen.y) / self.screen.height,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footer_and_screen_share_dpi_scaled_geometry() {
        let layout = ViewerLayout::new(800, 1600, 400, 800, 2.0);
        assert_eq!(layout.footer.height, 96.0);
        assert_eq!((layout.home.width, layout.home.height), (160.0, 64.0));
        assert_eq!(layout.screen.height, 1504.0);
        assert_eq!(layout.screen.width, 752.0);
        assert_eq!(layout.screen.y + layout.screen.height, layout.footer.y);
        assert!(layout.home_contains(400.0, 1552.0));
        assert_eq!(
            layout.screen_position(400.0, 752.0, false),
            Some((0.5, 0.5))
        );
        assert_eq!(layout.screen_position(400.0, 1552.0, false), None);
    }

    #[test]
    fn corners_are_visually_clipped_and_not_touch_targets() {
        let layout = ViewerLayout::new(400, 848, 400, 800, 1.0);
        assert_eq!(layout.screen_position(0.0, 0.0, false), None);
        assert_eq!(layout.screen_position(200.0, 0.0, false), Some((0.5, 0.0)));
        let clamped = layout.screen_position(-10.0, -10.0, true);
        assert!(matches!(clamped, Some((x,y)) if x > 0.0 && y > 0.0 && x < 0.12 && y < 0.12));
        assert!(!layout.home_contains(layout.home.x, layout.home.y));
    }

    #[test]
    fn layouts_remain_finite_and_bounded_across_sizes() {
        for width in [0, 1, 16, 200, 1200] {
            for height in [0, 1, 20, 400, 1800] {
                for dpi in [0.0, f64::NAN, 0.5, 1.0, 2.0, 3.0] {
                    let layout = ViewerLayout::new(width, height, 1184, 2576, dpi);
                    for rect in [layout.screen, layout.footer, layout.home] {
                        assert!(rect.x.is_finite() && rect.y.is_finite());
                        assert!(rect.x >= 0.0 && rect.y >= 0.0);
                        assert!(rect.width >= 0.0 && rect.height >= 0.0);
                        assert!(rect.x + rect.width <= f64::from(width) + 1e-8);
                        assert!(rect.y + rect.height <= f64::from(height) + 1e-8);
                    }
                    assert_eq!(layout.screen_position(f64::NAN, 0.0, true), None);
                }
            }
        }
    }
}
