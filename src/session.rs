use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use iphone_mirror_rs::device::{DeviceOptions, DeviceSession, HidChannels, OrientationSource};
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

// This seam keeps cancellation and cleanup testable without opening an iPhone.
trait InputSink: Send + 'static {
    fn send(&mut self, event: &HidEvent) -> impl std::future::Future<Output = Result<()>> + Send;
    fn close(&mut self) -> impl std::future::Future<Output = Result<()>> + Send;
}

impl InputSink for HidChannels {
    async fn send(&mut self, event: &HidEvent) -> Result<()> {
        HidChannels::send(self, event).await
    }
    async fn close(&mut self) -> Result<()> {
        HidChannels::close(self).await
    }
}

async fn input_worker(
    mut hid: impl InputSink,
    input: Arc<InputBus>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let result: Result<()> = async {
        loop {
            if *stop.borrow() { break; }
            tokio::select! {
                biased;
                _ = stop.changed() => break,
                _ = input.wake.notified() => {
                    while let Some(event) = input.pop()? {
                        if *stop.borrow() { return Ok(()); }
                        tokio::select! {
                            biased;
                            _ = stop.changed() => return Ok(()),
                            sent = tokio::time::timeout(Duration::from_secs(2), hid.send(&event)) => {
                                sent.context("input send deadline exceeded")??;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }.await;
    // Cancellation drops a possibly-partial send. HidChannels conservatively
    // tracks held state before writing, so close releases it in either case.
    let cleanup = tokio::time::timeout(Duration::from_secs(3), hid.close())
        .await
        .context("input cleanup deadline exceeded")
        .and_then(|result| result);
    result.and(cleanup)
}

#[derive(Default)]
struct DecodeProgress {
    packets: u64,
    decoded: u64,
    stalled_since: Option<Instant>,
}

impl DecodeProgress {
    /// An idle screen is allowed. Only continuous RTP without decoded output
    /// starts the recovery clock; RTCP keepalives do not count as RTP progress.
    fn observe(&mut self, now: Instant, packets: u64, decoded: u64) -> Duration {
        if decoded != self.decoded || packets == self.packets {
            self.stalled_since = None;
        } else {
            self.stalled_since.get_or_insert(now);
        }
        self.packets = packets;
        self.decoded = decoded;
        self.stalled_since
            .map_or(Duration::ZERO, |start| now.duration_since(start))
    }
}

const ORIENTATION_POLL_INTERVAL: Duration = Duration::from_millis(400);

trait OrientationPoll: Send + 'static {
    fn poll(&mut self) -> impl std::future::Future<Output = Result<u32>> + Send;
    fn reset(&mut self);
}

impl OrientationPoll for OrientationSource {
    async fn poll(&mut self) -> Result<u32> {
        OrientationSource::poll(self).await
    }
    fn reset(&mut self) {
        OrientationSource::reset(self);
    }
}

struct OrientationState {
    last: Option<u32>,
    delay: Duration,
    failed: bool,
}

impl Default for OrientationState {
    fn default() -> Self {
        Self {
            last: None,
            delay: ORIENTATION_POLL_INTERVAL,
            failed: false,
        }
    }
}

impl OrientationState {
    fn success(&mut self, value: u32) -> Option<u32> {
        self.delay = ORIENTATION_POLL_INTERVAL;
        self.failed = false;
        if (1..=4).contains(&value) && self.last != Some(value) {
            self.last = Some(value);
            Some(value)
        } else {
            None
        }
    }

    fn failure(&mut self) -> bool {
        self.delay = (self.delay * 2).min(Duration::from_secs(5));
        let first = !self.failed;
        self.failed = true;
        first
    }
}

async fn orientation_worker(
    mut source: impl OrientationPoll,
    mut stop: watch::Receiver<bool>,
    mut emit: impl FnMut(u32) -> bool,
) {
    let mut state = OrientationState::default();
    loop {
        if *stop.borrow() {
            break;
        }
        let outcome = tokio::select! {
            biased;
            _ = stop.changed() => break,
            outcome = tokio::time::timeout(Duration::from_secs(1), source.poll()) => outcome,
        };
        match outcome {
            Ok(Ok(value)) => {
                if let Some(value) = state.success(value) {
                    tracing::info!(orientation = value, "device interface orientation changed");
                    if !emit(value) {
                        break;
                    }
                }
            }
            _ => {
                source.reset();
                if state.failure() {
                    tracing::warn!(
                        "interface orientation query failed; retrying independently of video"
                    );
                }
            }
        }
        tokio::select! {
            biased;
            _ = stop.changed() => break,
            _ = tokio::time::sleep(state.delay) => {},
        }
    }
    // Dropping the source closes its dedicated SpringBoard service stream.
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
    // Keep the verified protocol order: iOS 27 rejected media startup with
    // error 9022 when HID was registered first. Register only after streaming.
    let stream = match session.start_video().await {
        Ok(stream) => stream,
        Err(error) => {
            let _ = session.stop().await;
            return Err(error);
        }
    };
    let hid = match session.open_input().await {
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

    let (input_stop, input_stop_rx) = watch::channel(false);
    let mut input_task = tokio::spawn(input_worker(
        hid,
        shared.input.clone(),
        input_stop_rx.clone(),
    ));
    let mut input_result = None;
    let orientation_task =
        shared
            .proxy
            .clone()
            .and_then(|proxy| match session.orientation_source() {
                Ok(source) => Some(tokio::spawn(orientation_worker(
                    source,
                    input_stop_rx,
                    move |value| {
                        proxy
                            .send_event(AppEvent::OrientationChanged(value))
                            .is_ok()
                    },
                ))),
                Err(_) => {
                    tracing::warn!(
                        "interface orientation service unavailable; video remains active"
                    );
                    None
                }
            });

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
    let mut recovery = tokio::time::interval(Duration::from_millis(500));
    recovery.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut progress = DecodeProgress::default();
    let mut first_frame = false;
    let mut reset_pending = false;
    let mut last_pli = Instant::now() - Duration::from_secs(1);
    let mut last_packet = Instant::now();
    let result: Result<()> = async {
        loop {
            tokio::select! {
                biased;
                _ = stop.changed() => break,
                finished = &mut input_task => {
                    input_result = Some(finished.context("input task failed").and_then(|result| result));
                    break;
                }
                _ = report.tick() => {
                    shared.metrics.report();
                    if last_packet.elapsed() > Duration::from_secs(10) { bail!("video packet deadline exceeded"); }
                    if decode.is_finished() { bail!("decoder worker ended"); }
                    let rr = receiver_report(stream.info.local_ssrc, stream.info.remote_ssrc,
                        depacketizer.stats().highest_sequence);
                    tokio::time::timeout(Duration::from_millis(250), stream.send_rtcp(&rr)).await
                        .context("RTCP receiver report send deadline exceeded")??;
                }
                _ = recovery.tick() => {
                    let now = Instant::now();
                    let stalled = progress.observe(now, depacketizer.stats().packets,
                        shared.metrics.decoded.load(Ordering::Relaxed));
                    if stalled >= Duration::from_secs(5) {
                        bail!("RTP continues but decoded frames stopped for five seconds");
                    }
                    if stalled >= Duration::from_secs(2) {
                        first_frame = false;
                        reset_pending = true;
                    }
                    // This timer runs even if every AU is incomplete or no AU
                    // arrives. Packet-triggered PLI alone cannot recover that case.
                    if !first_frame && last_pli.elapsed() >= Duration::from_millis(500) {
                        tokio::time::timeout(Duration::from_millis(250), stream.send_rtcp(
                            &picture_loss_indication(stream.info.local_ssrc, stream.info.remote_ssrc))).await
                            .context("keyframe request send deadline exceeded")??;
                        last_pli = now;
                    }
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
    let _ = input_stop.send(true);
    let input_cleanup = match input_result {
        Some(result) => result,
        None => match tokio::time::timeout(Duration::from_secs(4), &mut input_task).await {
            Ok(result) => result
                .context("input task failed")
                .and_then(|result| result),
            Err(_) => {
                // The worker has its own three-second cleanup deadline. Abort
                // and join only if it violated that bound; never detach it.
                input_task.abort();
                let _ = input_task.await;
                Err(anyhow::anyhow!("input worker shutdown deadline exceeded"))
            }
        },
    };
    if let Some(mut orientation_task) = orientation_task {
        match tokio::time::timeout(Duration::from_secs(2), &mut orientation_task).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => tracing::warn!("orientation task ended unexpectedly"),
            Err(_) => {
                orientation_task.abort();
                let _ = orientation_task.await;
                tracing::warn!("orientation task shutdown deadline exceeded");
            }
        }
    }
    let stream_cleanup = tokio::time::timeout(Duration::from_secs(5), session.stop()).await;
    let decoder_result = decode.await.context("decoder thread failed")?;
    shared.metrics.report();
    result?;
    input_cleanup?;
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

    struct FakeInput {
        started: Arc<Notify>,
        blocked: Arc<Notify>,
        closed: Arc<AtomicBool>,
        fail_send: bool,
    }

    impl InputSink for FakeInput {
        async fn send(&mut self, _: &HidEvent) -> Result<()> {
            self.started.notify_one();
            if self.fail_send {
                bail!("synthetic input write failure");
            }
            self.blocked.notified().await;
            Ok(())
        }
        async fn close(&mut self) -> Result<()> {
            self.closed.store(true, Ordering::Release);
            Ok(())
        }
    }

    #[tokio::test]
    async fn cancel_interrupts_blocked_input_and_releases_it() -> Result<()> {
        let bus = Arc::new(InputBus::new());
        let started = Arc::new(Notify::new());
        let closed = Arc::new(AtomicBool::new(false));
        let sink = FakeInput {
            started: started.clone(),
            blocked: Arc::new(Notify::new()),
            closed: closed.clone(),
            fail_send: false,
        };
        let (stop, receiver) = watch::channel(false);
        assert!(bus.push(HidEvent::Home { pressed: true }));
        let worker = tokio::spawn(input_worker(sink, bus, receiver));
        tokio::time::timeout(Duration::from_secs(1), started.notified()).await?;
        assert!(!worker.is_finished());
        // A blocked HID socket is an independent future; the caller can still
        // receive media and issue shutdown without waiting for its send timeout.
        stop.send(true)?;
        tokio::time::timeout(Duration::from_secs(1), worker).await???;
        assert!(closed.load(Ordering::Acquire));
        Ok(())
    }

    #[tokio::test]
    async fn input_write_failure_still_runs_cleanup() -> Result<()> {
        let bus = Arc::new(InputBus::new());
        let closed = Arc::new(AtomicBool::new(false));
        let sink = FakeInput {
            started: Arc::new(Notify::new()),
            blocked: Arc::new(Notify::new()),
            closed: closed.clone(),
            fail_send: true,
        };
        let (_stop, receiver) = watch::channel(false);
        assert!(bus.push(HidEvent::Home { pressed: true }));
        let result =
            tokio::time::timeout(Duration::from_secs(1), input_worker(sink, bus, receiver)).await?;
        assert_eq!(
            result.err().context("send must fail")?.to_string(),
            "synthetic input write failure"
        );
        assert!(closed.load(Ordering::Acquire));
        Ok(())
    }

    #[tokio::test]
    async fn input_overflow_still_runs_cleanup() -> Result<()> {
        let bus = Arc::new(InputBus::new());
        for _ in 0..129 {
            bus.push(HidEvent::Home { pressed: true });
        }
        let closed = Arc::new(AtomicBool::new(false));
        let sink = FakeInput {
            started: Arc::new(Notify::new()),
            blocked: Arc::new(Notify::new()),
            closed: closed.clone(),
            fail_send: false,
        };
        let (_stop, receiver) = watch::channel(false);
        assert!(
            tokio::time::timeout(Duration::from_secs(1), input_worker(sink, bus, receiver))
                .await?
                .is_err()
        );
        assert!(closed.load(Ordering::Acquire));
        Ok(())
    }

    #[test]
    fn watchdog_detects_continuous_rtp_without_decoding() {
        let mut progress = DecodeProgress::default();
        let start = Instant::now();
        for tick in 0..=10 {
            let stalled = progress.observe(start + Duration::from_millis(tick * 500), tick + 1, 0);
            assert_eq!(stalled, Duration::from_millis(tick * 500));
        }
        // A produced frame restores progress immediately.
        assert_eq!(
            progress.observe(start + Duration::from_secs(6), 12, 1),
            Duration::ZERO
        );
    }

    #[test]
    fn watchdog_does_not_treat_idle_screen_as_stalled_decoder() {
        let mut progress = DecodeProgress::default();
        let start = Instant::now();
        progress.observe(start, 1, 1);
        assert_eq!(
            progress.observe(start + Duration::from_secs(20), 1, 1),
            Duration::ZERO
        );
        // New activity after a long idle period gets a fresh recovery window.
        assert_eq!(
            progress.observe(start + Duration::from_secs(21), 2, 1),
            Duration::ZERO
        );
        assert_eq!(
            progress.observe(start + Duration::from_millis(21500), 3, 1),
            Duration::from_millis(500)
        );
        assert_eq!(
            progress.observe(start + Duration::from_secs(22), 3, 1),
            Duration::ZERO
        );
    }

    #[test]
    fn orientation_keeps_last_valid_value_and_deduplicates_changes() {
        let mut state = OrientationState::default();
        assert_eq!(state.success(1), Some(1));
        assert_eq!(state.success(1), None);
        assert_eq!(state.success(0), None);
        assert_eq!(state.success(5), None);
        assert_eq!(state.last, Some(1));
        for value in [2, 3, 4] {
            assert_eq!(state.success(value), Some(value));
        }
        assert_eq!(state.success(4), None);
    }

    #[test]
    fn orientation_retry_backoff_is_bounded_and_resets_after_success() {
        let mut state = OrientationState::default();
        assert!(state.failure());
        assert_eq!(state.delay, Duration::from_millis(800));
        for _ in 0..20 {
            assert!(!state.failure());
        }
        assert_eq!(state.delay, Duration::from_secs(5));
        assert_eq!(state.success(1), Some(1));
        assert_eq!(state.delay, ORIENTATION_POLL_INTERVAL);
        assert!(state.failure());
    }

    struct FakeOrientation {
        started: Arc<Notify>,
        resets: Arc<std::sync::atomic::AtomicUsize>,
        dropped: Arc<AtomicBool>,
        answers: std::collections::VecDeque<Result<u32>>,
    }

    impl OrientationPoll for FakeOrientation {
        async fn poll(&mut self) -> Result<u32> {
            self.started.notify_one();
            match self.answers.pop_front() {
                Some(answer) => answer,
                None => std::future::pending().await,
            }
        }
        fn reset(&mut self) {
            self.resets.fetch_add(1, Ordering::Relaxed);
        }
    }
    impl Drop for FakeOrientation {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::Release);
        }
    }

    #[tokio::test]
    async fn orientation_cancel_drops_blocked_service_without_waiting_for_timeout() -> Result<()> {
        let started = Arc::new(Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let source = FakeOrientation {
            started: started.clone(),
            resets: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            dropped: dropped.clone(),
            answers: std::collections::VecDeque::new(),
        };
        let (stop, receiver) = watch::channel(false);
        let task = tokio::spawn(orientation_worker(source, receiver, |_| true));
        tokio::time::timeout(Duration::from_secs(1), started.notified()).await?;
        stop.send(true)?;
        tokio::time::timeout(Duration::from_millis(250), task).await??;
        assert!(dropped.load(Ordering::Acquire));
        Ok(())
    }

    #[tokio::test]
    async fn orientation_failure_discards_service_then_recovers() -> Result<()> {
        let resets = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let dropped = Arc::new(AtomicBool::new(false));
        let source = FakeOrientation {
            started: Arc::new(Notify::new()),
            resets: resets.clone(),
            dropped: dropped.clone(),
            answers: [Err(anyhow::anyhow!("synthetic query failure")), Ok(3)]
                .into_iter()
                .collect(),
        };
        let (_stop, receiver) = watch::channel(false);
        let (sent, mut received) = mpsc::unbounded_channel();
        let task = tokio::spawn(orientation_worker(source, receiver, move |value| {
            let _ = sent.send(value);
            false // A closed UI ends polling and releases the dedicated service.
        }));
        tokio::time::timeout(Duration::from_secs(2), task).await??;
        assert_eq!(received.try_recv()?, 3);
        assert_eq!(resets.load(Ordering::Relaxed), 1);
        assert!(dropped.load(Ordering::Acquire));
        Ok(())
    }
}
