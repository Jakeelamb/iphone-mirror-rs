//! Fixed-size counters. Frame contents and input values never enter tracing.
use std::array;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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
    pub packets: AtomicU64,
    pub bytes: AtomicU64,
    pub access_units: AtomicU64,
    pub decoded: AtomicU64,
    pub replaced: AtomicU64,
    pub submitted: AtomicU64,
    pub decode: Histogram,
    pub receive_to_submit: Histogram,
}

impl Metrics {
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
            "pipeline counters; submission is not display scanout"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_cover_empty_bucket_edges_and_overflow() {
        let h = Histogram::default();
        assert_eq!(h.percentile_ms(95), 0);
        for us in [0, 999, 1000, 2000, 999_999] {
            h.record(Duration::from_micros(us));
        }
        assert_eq!(h.percentile_ms(40), 1);
        assert_eq!(h.percentile_ms(50), 2);
        assert_eq!(h.percentile_ms(100), 256);
        assert!((h.mean_ms() - 200.7996).abs() < 0.0001);
    }
}
