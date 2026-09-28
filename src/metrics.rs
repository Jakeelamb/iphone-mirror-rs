//! Fixed-size counters. Frame contents and input values never enter tracing.
use crate::video::DecodedFrame;
use std::array;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BUCKETS: usize = 256;

pub struct Histogram {
    buckets: [AtomicU64; BUCKETS],
    total_us: AtomicU64,
    count: AtomicU64,
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            buckets: array::from_fn(|_| AtomicU64::new(0)),
            total_us: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }
}

impl Histogram {
    pub fn samples(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    pub fn record(&self, elapsed: Duration) {
        let us = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
        let bucket = (us / 1000).min((BUCKETS - 1) as u64) as usize;
        self.buckets[bucket].fetch_add(1, Ordering::Relaxed);
        self.total_us.fetch_add(us, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn mean_ms(&self) -> f64 {
        let count = self.count.load(Ordering::Relaxed);
        if count == 0 {
            return 0.0;
        }
        self.total_us.load(Ordering::Relaxed) as f64 / count as f64 / 1000.0
    }

    /// Upper edge of a 1 ms bucket; 256 denotes the overflow bucket (>=255 ms).
    pub fn percentile_ms(&self, percentile: u64) -> u64 {
        let count = self.count.load(Ordering::Relaxed);
        if count == 0 {
            return 0;
        }
        let target = (count * percentile.clamp(1, 100)).div_ceil(100);
        let mut sum = 0;
        for (index, bucket) in self.buckets.iter().enumerate() {
            sum += bucket.load(Ordering::Relaxed);
            if sum >= target {
                return (index + 1) as u64;
            }
        }
        BUCKETS as u64
    }
}

#[derive(Default)]
pub struct Metrics {
    pub measure_stamp: AtomicBool,
    pub packets: AtomicU64,
    pub bytes: AtomicU64,
    pub access_units: AtomicU64,
    pub decoded: AtomicU64,
    pub replaced: AtomicU64,
    pub submitted: AtomicU64,
    pub render_attempts: AtomicU64,
    pub surface_timeouts: AtomicU64,
    pub look_resets: AtomicU64,
    pub clipped_mouse_events: AtomicU64,
    pub input_queue_high_water: AtomicU64,
    pub input_coalesced: AtomicU64,
    /// Latest retained input snapshot enqueue to local HID write start.
    pub input_queue_age: Histogram,
    /// Age of its queue slot, including any replaced motion snapshots.
    pub input_slot_age: Histogram,
    /// Successful local HID write duration; not a device acknowledgment.
    pub input_write: Histogram,
    pub input_enqueue_to_write_complete: Histogram,
    /// Access-unit handoff attempt to decoder dequeue. Includes waiting for
    /// bounded channel capacity, but excludes assembly and upstream queues.
    pub encoded_handoff: Histogram,
    /// Host texture write enqueue, including texture allocation when needed.
    pub upload: Histogram,
    pub surface_acquire: Histogram,
    /// Host render attempt duration, not GPU execution or display scanout.
    pub render_submit: Histogram,
    /// Distinct-frame successful submission spacing, excluding UI-only redraws.
    pub submit_interval: Histogram,
    pub decode: Histogram,
    pub receive_to_submit: Histogram,
    pub source_to_submit: Histogram,
    stamp_y: AtomicU64,
}

impl Metrics {
    /// Optional synthetic-page timing probe. It never saves pixels or text.
    /// Green/32 binary cells/magenta must span the whole encoded picture.
    pub fn record_source_stamp(&self, frame: &DecodedFrame) {
        if !self.measure_stamp.load(Ordering::Relaxed) || frame.width < 34 {
            return;
        }
        let x = |column: u32| ((u64::from(frame.width) * u64::from(2 * column + 1)) / 68) as u32;
        let markers = |y| {
            matches!((frame.rgb_at(x(0),y), frame.rgb_at(x(33),y)),
                (Some([r,g,b]),Some([rr,gg,bb])) if r<100 && g>150 && b<100 && rr>170 && gg<100 && bb>170)
        };
        let cached = self.stamp_y.load(Ordering::Relaxed) as u32;
        let y = if cached < frame.height && markers(cached) {
            Some(cached)
        } else {
            (0..frame.height).step_by(4).find(|&y| markers(y))
        };
        let Some(y) = y else {
            return;
        };
        self.stamp_y.store(u64::from(y), Ordering::Relaxed);
        let mut value = 0_u32;
        for column in 1..33 {
            let Some(luma) = frame.luma_at(x(column), y) else {
                return;
            };
            value = (value << 1) | u32::from(luma > 128);
        }
        let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
            return;
        };
        let age = (now.as_millis() as u32).wrapping_sub(value);
        if age <= 5000 {
            self.source_to_submit
                .record(Duration::from_millis(u64::from(age)));
        }
    }

    pub fn report(&self) {
        tracing::info!(
            packets = self.packets.load(Ordering::Relaxed),
            bytes = self.bytes.load(Ordering::Relaxed),
            access_units = self.access_units.load(Ordering::Relaxed),
            decoded = self.decoded.load(Ordering::Relaxed),
            replaced_decoded_frames = self.replaced.load(Ordering::Relaxed),
            submitted = self.submitted.load(Ordering::Relaxed),
            decode_mean_ms = self.decode.mean_ms(),
            receive_to_submit_mean_ms = self.receive_to_submit.mean_ms(),
            receive_to_submit_p50_ms = self.receive_to_submit.percentile_ms(50),
            receive_to_submit_p95_ms = self.receive_to_submit.percentile_ms(95),
            receive_to_submit_p99_ms = self.receive_to_submit.percentile_ms(99),
            stamp_samples = self.source_to_submit.count.load(Ordering::Relaxed),
            source_to_submit_mean_ms = self.source_to_submit.mean_ms(),
            source_to_submit_p50_ms = self.source_to_submit.percentile_ms(50),
            source_to_submit_p95_ms = self.source_to_submit.percentile_ms(95),
            "pipeline counters; submission is not display scanout"
        );
        tracing::info!(
            input_queue_high_water = self.input_queue_high_water.load(Ordering::Relaxed),
            input_coalesced = self.input_coalesced.load(Ordering::Relaxed),
            look_resets = self.look_resets.load(Ordering::Relaxed),
            clipped_mouse_events = self.clipped_mouse_events.load(Ordering::Relaxed),
            render_attempts = self.render_attempts.load(Ordering::Relaxed),
            surface_timeouts = self.surface_timeouts.load(Ordering::Relaxed),
            "bounded input and render counters"
        );
        // Each event has a fixed label and numeric fields only. Histograms use
        // 1 ms bucket upper edges; 256 is overflow (>=255 ms), not an exact age.
        for (stage, histogram) in [
            ("input_queue_age", &self.input_queue_age),
            ("input_slot_age", &self.input_slot_age),
            ("input_write", &self.input_write),
            (
                "input_enqueue_to_write_complete",
                &self.input_enqueue_to_write_complete,
            ),
            ("encoded_handoff", &self.encoded_handoff),
            ("upload", &self.upload),
            ("surface_acquire", &self.surface_acquire),
            ("render_submit", &self.render_submit),
            ("submit_interval", &self.submit_interval),
        ] {
            tracing::info!(
                stage,
                samples = histogram.samples(),
                mean_ms = histogram.mean_ms(),
                p95_ms = histogram.percentile_ms(95),
                p99_ms = histogram.percentile_ms(99),
                "host stage timing; write completion is not phone application or scanout"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_cover_empty_bucket_edges_and_overflow() {
        let h = Histogram::default();
        assert_eq!(h.samples(), 0);
        assert_eq!(h.percentile_ms(95), 0);
        for us in [0, 999, 1000, 2000, 999_999] {
            h.record(Duration::from_micros(us));
        }
        assert_eq!(h.percentile_ms(40), 1);
        assert_eq!(h.percentile_ms(50), 2);
        assert_eq!(h.percentile_ms(100), 256);
        assert_eq!(h.samples(), 5);
        assert!((h.mean_ms() - 200.7996).abs() < 0.0001);
    }
}
