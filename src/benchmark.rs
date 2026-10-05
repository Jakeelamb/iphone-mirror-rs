//! Final, content-free run summaries. No per-frame serialization or allocation.
use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use iphone_mirror_rs::metrics::{Histogram, Metrics};
use serde_json::{Value, json};

use crate::options::Options;

pub struct Benchmark {
    file: File,
    started: Instant,
    started_unix_seconds: u64,
    requested: Value,
}

impl Benchmark {
    pub fn new(path: &Path, options: &Options) -> Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .context("create new benchmark report; existing reports are never overwritten")?;
        Ok(Self {
            file,
            started: Instant::now(),
            started_unix_seconds: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            requested: json!({
                "transport": format!("{:?}", options.device.connection),
                "decoder": format!("{:?}", options.decoder),
                "renderer_gpu": format!("{:?}", options.presentation.gpu),
                "present_mode": format!("{:?}", options.presentation.mode),
                "frame_latency_hint": options.presentation.frame_latency,
                "pre_present_notify": options.presentation.pre_present_notify,
                "headless": options.headless,
                "duration_seconds": options.duration.map(|d| d.as_secs_f64()),
            }),
        })
    }

    pub fn finish(mut self, metrics: &Metrics, success: bool) -> Result<()> {
        let context = metrics
            .run_context
            .lock()
            .map_err(|_| anyhow::anyhow!("benchmark context unavailable"))?;
        let report = summary(metrics, self.started.elapsed().as_secs_f64(), success);
        let report = json!({
            "schema_version": 1,
            "package_version": env!("CARGO_PKG_VERSION"),
            "started_unix_seconds": self.started_unix_seconds,
            "requested": self.requested,
            "observed": *context,
            "results": report,
            "measurement_scope": "Full session including startup and shutdown; no automatic warmup exclusion. Host submission is not scanout. Input write is not phone acknowledgment. No end-to-end latency claim.",
        });
        serde_json::to_writer_pretty(&mut self.file, &report)?;
        self.file.sync_all().context("flush benchmark report")
    }
}

fn histogram(value: &Histogram) -> Value {
    if value.samples() == 0 {
        return Value::Null;
    }
    json!({ "samples": value.samples(), "mean_ms": value.mean_ms(),
        "p50_ms": value.percentile_ms(50), "p95_ms": value.percentile_ms(95),
        "p99_ms": value.percentile_ms(99), "bucket_ms": 1,
        "overflow_marker_ms": 256 })
}

fn summary(metrics: &Metrics, elapsed: f64, success: bool) -> Value {
    let count = |counter: &std::sync::atomic::AtomicU64| counter.load(Ordering::Relaxed);
    let submitted = count(&metrics.submitted);
    let spacing = &metrics.submit_interval;
    json!({
        "session_success": success,
        "video_observed": count(&metrics.decoded) > 0,
        "presentation_observed": submitted > 0,
        "elapsed_seconds": elapsed,
        "decoded": count(&metrics.decoded), "submitted": submitted,
        "replaced_frames": count(&metrics.replaced),
        "mean_submission_cadence_fps": if spacing.samples() > 0 && spacing.mean_ms() > 0.0 { Some(1000.0 / spacing.mean_ms()) } else { None },
        "submission_gaps_33ms": count(&metrics.submission_gaps_33ms),
        "submission_gaps_50ms": count(&metrics.submission_gaps_50ms),
        "feedback_sent": count(&metrics.feedback_sent),
        "feedback_timeouts": count(&metrics.feedback_timeouts),
        "surface_timeouts": count(&metrics.surface_timeouts),
        "timings": {
            "decode": histogram(&metrics.decode),
            "encoded_handoff": histogram(&metrics.encoded_handoff),
            "upload": histogram(&metrics.upload),
            "surface_acquire": histogram(&metrics.surface_acquire),
            "render_submit": histogram(&metrics.render_submit),
            "submit_interval": histogram(spacing),
            "receive_to_submit": histogram(&metrics.receive_to_submit),
            "source_to_submit": histogram(&metrics.source_to_submit),
            "input_enqueue_to_write_complete": histogram(&metrics.input_enqueue_to_write_complete),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn absent_measurements_are_null_and_failures_are_distinct_from_observation() -> Result<()> {
        let metrics = Metrics::default();
        let empty = summary(&metrics, 2.0, false);
        assert_eq!(empty["video_observed"], false);
        assert!(empty["timings"]["decode"].is_null());
        assert!(empty["mean_submission_cadence_fps"].is_null());
        metrics.decoded.store(3, Ordering::Relaxed);
        metrics.submitted.store(3, Ordering::Relaxed);
        for ms in [10, 50] {
            metrics.record_submission_interval(Duration::from_millis(ms));
        }
        let report = summary(&metrics, 3.0, false);
        assert_eq!(report["session_success"], false);
        assert_eq!(report["presentation_observed"], true);
        assert_eq!(report["submission_gaps_50ms"], 1);
        assert_eq!(report["timings"]["submit_interval"]["p99_ms"], 51);
        assert!(
            (report["mean_submission_cadence_fps"]
                .as_f64()
                .context("missing cadence")?
                - 1000.0 / 30.0)
                .abs()
                < 0.001
        );
        Ok(())
    }

    #[test]
    fn report_is_private_preserves_existing_files_and_records_failed_runs() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let path =
            std::env::temp_dir().join(format!("mirror-benchmark-{}.json", uuid::Uuid::new_v4()));
        let options = Options::parse_from(["--duration", "1"].into_iter().map(str::to_owned))?
            .context("missing options")?;
        let report = Benchmark::new(&path, &options)?;
        assert!(Benchmark::new(&path, &options).is_err());
        let metrics = Metrics::default();
        metrics.set_context("transport", "usb".to_owned());
        report.finish(&metrics, false)?;
        let contents: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        assert_eq!(contents["observed"]["transport"], "usb");
        assert_eq!(contents["results"]["session_success"], false);
        assert_eq!(contents["results"]["presentation_observed"], false);
        assert_eq!(
            std::fs::metadata(&path)?.permissions().mode() & 0o777,
            0o600
        );
        std::fs::remove_file(path)?;
        Ok(())
    }
}
