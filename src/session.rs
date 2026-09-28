use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use iphone_mirror_rs::device::{DeviceOptions, DeviceSession};
use iphone_mirror_rs::input::{HidEvent, InputQueue};
use iphone_mirror_rs::metrics::Metrics;
use iphone_mirror_rs::rtp::{HevcDepacketizer, picture_loss_indication, receiver_report};
use iphone_mirror_rs::video::{DecodeMode, Decoder, LatestFrame};
use tokio::sync::{Notify, mpsc, watch};
use winit::event_loop::EventLoopProxy;

use crate::app::AppEvent;

pub struct InputBus {
    queue: Mutex<InputQueue>,
    failed: AtomicBool,
    wake: Notify,
}

impl InputBus {
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(InputQueue::new(128)),
            failed: AtomicBool::new(false),
            wake: Notify::new(),
        }
    }

    pub fn push(&self, event: HidEvent) -> bool {
        let success = self
            .queue
            .lock()
            .is_ok_and(|mut queue| queue.try_push(event).is_ok());
        if !success {
            self.failed.store(true, Ordering::Release);
        }
        self.wake.notify_one();
        success
    }

    pub(crate) fn pop(&self) -> Result<Option<HidEvent>> {
        if self.failed.load(Ordering::Acquire) {
            bail!("input queue overflow; ending session to release held input");
        }
        Ok(self
            .queue
            .lock()
            .map_err(|_| anyhow::anyhow!("input queue unavailable"))?
            .pop())
    }
}

struct EncodedFrame {
    bytes: Vec<u8>,
    received_at: Instant,
    reset: bool,
}

pub struct SessionShared {
    pub input: Arc<InputBus>,
    pub latest: Arc<LatestFrame>,
    pub metrics: Arc<Metrics>,
    pub proxy: Option<EventLoopProxy<AppEvent>>,
}

pub async fn run(
    options: DeviceOptions,
    mode: DecodeMode,
    mut stop: watch::Receiver<bool>,
    shared: SessionShared,
) -> Result<()> {
    // Device/GPU initialization must finish before the phone starts producing
    // frames. Otherwise a healthy stream fills the bounded queue during startup.
    let mut decoder = tokio::task::spawn_blocking(move || Decoder::new(mode))
        .await
        .context("decoder initialization task failed")??;
    tracing::info!("connecting native device transport");
    let mut session =
        tokio::time::timeout(Duration::from_secs(35), DeviceSession::connect(&options))
            .await
            .context("device connection deadline exceeded")??;
    let stream = match session.start_video().await {
        Ok(stream) => stream,
        Err(error) => {
            let _ = session.stop().await;
            return Err(error);
        }
    };
    let mut hid = match session.open_input().await {
        Ok(hid) => hid,
        Err(error) => {
            let _ = session.stop().await;
            return Err(error);
        }
    };
    tracing::info!("native video and HID connected");
    if let Some(proxy) = &shared.proxy {
        let _ = proxy.send_event(AppEvent::Connected);
    }

    let (encoded_tx, mut encoded_rx) = mpsc::channel::<EncodedFrame>(4);
    let (free_tx, mut free_rx) = mpsc::channel::<Vec<u8>>(6);
    let metrics = shared.metrics.clone();
    let latest = shared.latest.clone();
    let proxy = shared.proxy.clone();
    let decode = tokio::task::spawn_blocking(move || -> Result<()> {
        while let Some(packet) = encoded_rx.blocking_recv() {
            if packet.reset {
                decoder.reset();
            }
            let started = Instant::now();
            decoder.decode(&packet.bytes, packet.received_at, |frame| {
                metrics.decoded.fetch_add(1, Ordering::Relaxed);
                if latest.publish(frame) {
                    metrics.replaced.fetch_add(1, Ordering::Relaxed);
                } else if let Some(proxy) = &proxy {
                    let _ = proxy.send_event(AppEvent::FrameReady);
                }
            })?;
            metrics.decode.record(started.elapsed());
            let _ = free_tx.try_send(packet.bytes);
        }
        Ok(())
    });

    let mut depacketizer = HevcDepacketizer::default();
    let mut report = tokio::time::interval(Duration::from_secs(1));
    report.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut first_frame = false;
    let mut reset_pending = false;
    let mut last_pli = Instant::now() - Duration::from_secs(1);
    let mut last_packet = Instant::now();
    let result: Result<()> = async {
        loop {
            tokio::select! {
                biased;
                _ = stop.changed() => break,
                _ = shared.input.wake.notified() => {
                    while let Some(event) = shared.input.pop()? {
                        tokio::time::timeout(Duration::from_secs(2), hid.send(&event)).await
                            .context("input send deadline exceeded")??;
                    }
                }
                _ = report.tick() => {
                    shared.metrics.report();
                    if last_packet.elapsed() > Duration::from_secs(10) { bail!("video packet deadline exceeded"); }
                    if decode.is_finished() { bail!("decoder worker ended"); }
                    let rr = receiver_report(stream.info.local_ssrc, stream.info.remote_ssrc,
                        depacketizer.stats().highest_sequence);
                    stream.send_rtcp(&rr).await?;
                }
                packet = stream.recv() => {
                    let packet = packet?;
                    last_packet = Instant::now();
                    shared.metrics.packets.fetch_add(1, Ordering::Relaxed);
                    shared.metrics.bytes.fetch_add(packet.data.len() as u64, Ordering::Relaxed);
                    let au = match depacketizer.push(&packet.data) {
                        Ok(Some(au)) => au,
                        Ok(None) => continue,
                        Err(_) => { tracing::debug!("malformed or incomplete media packet rejected"); continue; }
                    };
                    if au.discontinuity { first_frame = false; reset_pending = true; }
                    if !first_frame && !au.keyframe {
                        if last_pli.elapsed() > Duration::from_millis(500) {
                            stream.send_rtcp(&picture_loss_indication(stream.info.local_ssrc, stream.info.remote_ssrc)).await?;
                            last_pli = Instant::now();
                        }
                        continue;
                    }
                    first_frame = true;
                    shared.metrics.access_units.fetch_add(1, Ordering::Relaxed);
                    let mut bytes = free_rx.try_recv().unwrap_or_default();
                    bytes.clear();
                    bytes.extend_from_slice(au.data);
                    tokio::time::timeout(Duration::from_millis(100), encoded_tx.send(EncodedFrame {
                        bytes, received_at: last_packet, reset: reset_pending,
                    })).await.context("decoder stalled; bounded packet handoff timed out")?
                        .context("decoder worker ended")?;
                    reset_pending = false;
                }
            }
        }
        Ok(())
    }.await;
    drop(encoded_tx);
    // Services must stop while the authenticated tunnel is still alive.
    let input_cleanup = tokio::time::timeout(Duration::from_secs(3), hid.close()).await;
    let stream_cleanup = tokio::time::timeout(Duration::from_secs(5), session.stop()).await;
    let decoder_result = decode.await.context("decoder thread failed")?;
    shared.metrics.report();
    result?;
    input_cleanup.context("input cleanup deadline exceeded")??;
    stream_cleanup.context("stream cleanup deadline exceeded")??;
    decoder_result
}

#[cfg(test)]
mod input_tests {
    use super::*;
    #[test]
    fn overflowing_transitions_fail_session_instead_of_dropping_release() {
        let bus = InputBus::new();
        for i in 0..128 {
            assert!(bus.push(HidEvent::Home {
                pressed: i % 2 == 0
            }));
        }
        assert!(!bus.push(HidEvent::Home { pressed: false }));
        assert!(bus.pop().is_err());
    }
}
