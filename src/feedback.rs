//! Coalesced RTCP work. A blocked feedback send never blocks video reception.
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use iphone_mirror_rs::device::VideoStream;
use iphone_mirror_rs::metrics::Metrics;
use iphone_mirror_rs::rtp::{picture_loss_indication, receiver_report};
use std::sync::atomic::Ordering;
use tokio::sync::watch;

#[derive(Clone, Copy, Default)]
pub struct Request {
    pub sequence: u32,
    pub keyframe_generation: u64,
}

pub trait Sink: Send + Sync + 'static {
    fn send(&self, packet: &[u8]) -> impl std::future::Future<Output = Result<()>> + Send;
}

impl Sink for Arc<VideoStream> {
    async fn send(&self, packet: &[u8]) -> Result<()> {
        self.send_rtcp(packet).await
    }
}

pub async fn worker(
    sink: impl Sink,
    local: u32,
    remote: u32,
    mut requests: watch::Receiver<Request>,
    mut stop: watch::Receiver<bool>,
    metrics: Arc<Metrics>,
    deadline: Duration,
) -> Result<()> {
    let mut failures = 0;
    let mut sent_keyframe = 0;
    loop {
        if *stop.borrow() {
            return Ok(());
        }
        tokio::select! {
            biased;
            _ = stop.changed() => return Ok(()),
            changed = requests.changed() => if changed.is_err() { return Ok(()); },
        }
        let request = *requests.borrow_and_update();
        let rr = receiver_report(local, remote, request.sequence);
        let pli = picture_loss_indication(local, remote);
        let mut timed_out = false;
        for packet in [
            Some(rr.as_slice()),
            (request.keyframe_generation != sent_keyframe).then_some(pli.as_slice()),
        ]
        .into_iter()
        .flatten()
        {
            let outcome = tokio::select! {
                biased;
                _ = stop.changed() => return Ok(()),
                outcome = tokio::time::timeout(deadline, sink.send(packet)) => outcome,
            };
            match outcome {
                Ok(result) => {
                    result.context("RTCP feedback transport failed")?;
                    metrics.feedback_sent.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    metrics.feedback_timeouts.fetch_add(1, Ordering::Relaxed);
                    failures += 1;
                    tracing::warn!(
                        consecutive = failures,
                        "RTCP feedback send timed out; video receiver remains independent"
                    );
                    if failures >= 3 {
                        bail!("RTCP feedback stalled for three consecutive attempts");
                    }
                    timed_out = true;
                    break;
                }
            }
        }
        if !timed_out {
            failures = 0;
            sent_keyframe = request.keyframe_generation;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, AtomicUsize};
    use tokio::sync::Notify;

    struct Delayed {
        calls: Arc<AtomicUsize>,
        started: Arc<Notify>,
        blocked_calls: usize,
        last_sequence: Arc<AtomicU32>,
    }
    impl Sink for Delayed {
        async fn send(&self, packet: &[u8]) -> Result<()> {
            if packet.len() == 44 {
                let sequence = u32::from_be_bytes(packet[16..20].try_into()?);
                self.last_sequence.store(sequence, Ordering::Relaxed);
            }
            let call = self.calls.fetch_add(1, Ordering::Relaxed);
            self.started.notify_one();
            if call < self.blocked_calls {
                std::future::pending::<()>().await;
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn blocked_feedback_is_coalesced_and_transient_timeout_recovers() -> Result<()> {
        let (tx, rx) = watch::channel(Request::default());
        let (stop, stopped) = watch::channel(false);
        let calls = Arc::new(AtomicUsize::new(0));
        let last_sequence = Arc::new(AtomicU32::new(0));
        let started = Arc::new(Notify::new());
        let metrics = Arc::new(Metrics::default());
        let task = tokio::spawn(worker(
            Delayed {
                calls: calls.clone(),
                started: started.clone(),
                blocked_calls: 1,
                last_sequence: last_sequence.clone(),
            },
            1,
            2,
            rx,
            stopped,
            metrics.clone(),
            Duration::from_millis(30),
        ));
        tx.send_replace(Request {
            sequence: 1,
            keyframe_generation: 0,
        });
        tokio::time::timeout(Duration::from_secs(1), started.notified()).await?;
        // This is the same nonblocking publication used by the video loop.
        for sequence in 2..1000 {
            tx.send_replace(Request {
                sequence,
                keyframe_generation: 0,
            });
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        tokio::time::timeout(Duration::from_secs(1), started.notified()).await?;
        stop.send(true)?;
        task.await??;
        assert_eq!(metrics.feedback_timeouts.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.feedback_sent.load(Ordering::Relaxed), 1);
        assert_eq!(last_sequence.load(Ordering::Relaxed), 999);
        Ok(())
    }

    struct Recording {
        kinds: Arc<std::sync::Mutex<Vec<u8>>>,
        sent: Arc<Notify>,
    }

    impl Sink for Recording {
        async fn send(&self, packet: &[u8]) -> Result<()> {
            self.kinds
                .lock()
                .map_err(|_| anyhow::anyhow!("test sink poisoned"))?
                .push(packet[1]);
            self.sent.notify_one();
            Ok(())
        }
    }

    #[tokio::test]
    async fn keyframe_generation_survives_reports_without_duplicate_requests() -> Result<()> {
        let (tx, rx) = watch::channel(Request::default());
        let (stop, stopped) = watch::channel(false);
        let kinds = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sent = Arc::new(Notify::new());
        let task = tokio::spawn(worker(
            Recording {
                kinds: kinds.clone(),
                sent: sent.clone(),
            },
            1,
            2,
            rx,
            stopped,
            Arc::new(Metrics::default()),
            Duration::from_millis(30),
        ));
        for (generation, expected) in [(1, 2), (1, 3), (2, 5)] {
            tx.send_replace(Request {
                sequence: expected as u32,
                keyframe_generation: generation,
            });
            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    let count = kinds
                        .lock()
                        .map_err(|_| anyhow::anyhow!("test sink poisoned"))?
                        .len();
                    if count >= expected {
                        return Ok::<(), anyhow::Error>(());
                    }
                    sent.notified().await;
                }
            })
            .await??;
        }
        stop.send(true)?;
        task.await??;
        assert_eq!(
            *kinds
                .lock()
                .map_err(|_| anyhow::anyhow!("test sink poisoned"))?,
            [201, 206, 201, 201, 206]
        );
        Ok(())
    }

    #[tokio::test]
    async fn shutdown_interrupts_a_blocked_send() -> Result<()> {
        let (tx, rx) = watch::channel(Request::default());
        let (stop, stopped) = watch::channel(false);
        let started = Arc::new(Notify::new());
        let metrics = Arc::new(Metrics::default());
        let task = tokio::spawn(worker(
            Delayed {
                calls: Arc::new(AtomicUsize::new(0)),
                started: started.clone(),
                blocked_calls: usize::MAX,
                last_sequence: Arc::new(AtomicU32::new(0)),
            },
            1,
            2,
            rx,
            stopped,
            metrics.clone(),
            Duration::from_secs(30),
        ));
        tx.send_replace(Request {
            sequence: 1,
            keyframe_generation: 1,
        });
        tokio::time::timeout(Duration::from_secs(1), started.notified()).await?;
        stop.send(true)?;
        tokio::time::timeout(Duration::from_millis(100), task).await???;
        assert_eq!(metrics.feedback_timeouts.load(Ordering::Relaxed), 0);
        Ok(())
    }

    #[tokio::test]
    async fn persistent_feedback_stall_is_reported() -> Result<()> {
        let (tx, rx) = watch::channel(Request::default());
        let (_stop, stopped) = watch::channel(false);
        let started = Arc::new(Notify::new());
        let metrics = Arc::new(Metrics::default());
        let task = tokio::spawn(worker(
            Delayed {
                calls: Arc::new(AtomicUsize::new(0)),
                started: started.clone(),
                blocked_calls: usize::MAX,
                last_sequence: Arc::new(AtomicU32::new(0)),
            },
            1,
            2,
            rx,
            stopped,
            metrics.clone(),
            Duration::from_millis(10),
        ));
        for sequence in 1..=3 {
            tx.send_replace(Request {
                sequence,
                keyframe_generation: 0,
            });
            tokio::time::timeout(Duration::from_secs(1), started.notified()).await?;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(task.await?.is_err());
        assert_eq!(metrics.feedback_timeouts.load(Ordering::Relaxed), 3);
        Ok(())
    }
}
