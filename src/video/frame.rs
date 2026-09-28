use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Layout {
    Planar,
    Nv12,
}

#[derive(Default)]
pub(super) struct Plane {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub(super) type PlanePool = Arc<Mutex<Vec<[Plane; 3]>>>;

/// A decoded picture. Storage returns to the decoder's pool when dropped.
pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    pub received_at: Instant,
    pub decoded_at: Instant,
    pub(super) layout: Layout,
    pub(super) full_range: bool,
    pub(super) bt709: bool,
    pub(super) planes: [Plane; 3],
    pub(super) pool: PlanePool,
}

impl DecodedFrame {
    pub fn luma_at(&self, x: u32, y: u32) -> Option<u8> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.planes[0]
            .bytes
            .get((y * self.width + x) as usize)
            .copied()
    }

    /// Samples one pixel for opt-in synthetic latency markers; the normal
    /// renderer converts whole pictures in its GPU shader.
    pub fn rgb_at(&self, x: u32, y: u32) -> Option<[u8; 3]> {
        let mut luma = f32::from(self.luma_at(x, y)?) / 255.0;
        let chroma_x = x / 2;
        let chroma_y = y / 2;
        let first = &self.planes[1];
        let index = (chroma_y * first.width
            + chroma_x * if self.layout == Layout::Nv12 { 2 } else { 1 })
            as usize;
        let mut u = f32::from(*first.bytes.get(index)?) / 255.0 - 128.0 / 255.0;
        let mut v = f32::from(if self.layout == Layout::Nv12 {
            *first.bytes.get(index + 1)?
        } else {
            *self.planes[2].bytes.get(index)?
        }) / 255.0
            - 128.0 / 255.0;
        if !self.full_range {
            luma = (luma - 16.0 / 255.0) * 255.0 / 219.0;
            u *= 255.0 / 224.0;
            v *= 255.0 / 224.0;
        }
        let [rv, gu, gv, bu] = if self.bt709 {
            [1.5748, -0.187324, -0.468124, 1.8556]
        } else {
            [1.402, -0.344136, -0.714136, 1.772]
        };
        Some(
            [luma + rv * v, luma + gu * u + gv * v, luma + bu * u]
                .map(|component| (component.clamp(0.0, 1.0) * 255.0).round() as u8),
        )
    }
}

impl Drop for DecodedFrame {
    fn drop(&mut self) {
        let mut pool = self.pool.lock().unwrap_or_else(|error| error.into_inner());
        // Rendering and decoding normally need three sets. Bound retained memory
        // even if a caller holds frames far longer than the presentation path.
        if pool.len() < 4 {
            pool.push(std::mem::take(&mut self.planes));
        }
    }
}

/// Only complete decoded pictures may be replaced: HEVC packets keep their order.
#[derive(Default)]
pub struct LatestFrame {
    frame: Mutex<Option<DecodedFrame>>,
}

impl LatestFrame {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true when an older, unpresented decoded picture was replaced.
    pub fn publish(&self, frame: DecodedFrame) -> bool {
        self.frame
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .replace(frame)
            .is_some()
    }

    pub fn take(&self) -> Option<DecodedFrame> {
        self.frame
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn mailbox_replaces_only_decoded_frames_and_recycles_storage() {
        let pool = Arc::new(Mutex::new(Vec::new()));
        let make_frame = |width| DecodedFrame {
            width,
            height: 2,
            received_at: Instant::now(),
            decoded_at: Instant::now(),
            layout: Layout::Planar,
            full_range: false,
            bt709: true,
            planes: Default::default(),
            pool: pool.clone(),
        };
        let mailbox = LatestFrame::new();
        assert!(!mailbox.publish(make_frame(2)));
        assert!(mailbox.publish(make_frame(4)));
        assert_eq!(mailbox.take().map(|frame| frame.width), Some(4));
        assert!(mailbox.take().is_none());
        assert_eq!(pool.lock().expect("pool").len(), 2);
    }
}
