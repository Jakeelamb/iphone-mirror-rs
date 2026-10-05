use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use iphone_mirror_rs::device::{DeviceOptions, DeviceSession, HidChannels, OrientationSource};
use iphone_mirror_rs::input::{HidEvent, InputQueue, QueuedInput, TouchPhase};
use iphone_mirror_rs::metrics::{Metrics, RateWindow};
use iphone_mirror_rs::rtp::HevcDepacketizer;
use iphone_mirror_rs::video::{DecodeMode, Decoder, LatestFrame};
use tokio::sync::{Notify, mpsc, watch};
use winit::event_loop::EventLoopProxy;

use crate::app::AppEvent;

pub struct InputBus {
    queue: Mutex<InputQueue>,
    metrics: Arc<Metrics>,
    failed: AtomicBool,
    wake: Notify,
    reset: Notify,
    reset_generation: AtomicU64,
}

impl InputBus {
    pub fn new(metrics: Arc<Metrics>) -> Self {
        Self {
            queue: Mutex::new(InputQueue::new(128)),
            metrics,
            failed: AtomicBool::new(false),
            wake: Notify::new(),
            reset: Notify::new(),
            reset_generation: AtomicU64::new(0),
        }
    }

    pub fn push(&self, event: HidEvent) -> bool {
        self.push_at(event, Instant::now())
    }

    fn push_at(&self, event: HidEvent, now: Instant) -> bool {
        let success = self.queue.lock().is_ok_and(|mut queue| {
            let Ok(coalesced) = queue.try_push_at(event, now) else {
                return false;
            };
            if coalesced {
                self.metrics.input_coalesced.fetch_add(1, Ordering::Relaxed);
            }
            self.metrics
                .input_queue_high_water
                .fetch_max(queue.len() as u64, Ordering::Relaxed);
            true
        });
        if !success {
            self.failed.store(true, Ordering::Release);
        }
        self.wake.notify_one();
        success
    }

    #[cfg(test)]
    pub(crate) fn pop(&self) -> Result<Option<HidEvent>> {
        Ok(self.pop_with_generation()?.map(|(queued, _)| queued.event))
    }

    pub fn cancel_game(&self, timestamp: u64) -> bool {
        let now = Instant::now();
        let success = self.queue.lock().is_ok_and(|mut queue| {
            let success = queue.cancel_touches_at(timestamp, now).is_ok();
            self.metrics
                .input_queue_high_water
                .fetch_max(queue.len() as u64, Ordering::Relaxed);
            self.reset_generation.fetch_add(1, Ordering::AcqRel);
            success
        });
        if !success {
            self.failed.store(true, Ordering::Release);
        }
        self.reset.notify_one();
        self.wake.notify_one();
        success
    }

    fn pop_with_generation(&self) -> Result<Option<(QueuedInput, u64)>> {
        if self.failed.load(Ordering::Acquire) {
            bail!("input queue overflow; ending session to release held input");
        }
        let mut queue = self
            .queue
            .lock()
            .map_err(|_| anyhow::anyhow!("input queue unavailable"))?;
        let generation = self.reset_generation.load(Ordering::Acquire);
        Ok(queue.pop_timed().map(|event| (event, generation)))
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
    hid: impl InputSink,
    input: Arc<InputBus>,
    stop: watch::Receiver<bool>,
) -> Result<()> {
    input_worker_with_clock(hid, input, stop, Instant::now).await
}

async fn input_worker_with_clock(
    mut hid: impl InputSink,
    input: Arc<InputBus>,
    mut stop: watch::Receiver<bool>,
    now: impl Fn() -> Instant + Send,
) -> Result<()> {
    let result: Result<()> = async {
        loop {
            if *stop.borrow() { break; }
            tokio::select! {
                biased;
                _ = stop.changed() => break,
                _ = input.wake.notified() => {
                    while let Some((queued, generation)) = input.pop_with_generation()? {
                        let event = queued.event;
                        if *stop.borrow() { return Ok(()); }
                        if matches!(event, HidEvent::Touch { .. })
                            && input.reset_generation.load(Ordering::Acquire) != generation
                        {
                            continue;
                        }
                        // The successful write future ends at the local
                        // transport. It does not acknowledge phone application.
                        let send = async {
                            let started = now();
                            input.metrics.input_queue_age.record(started.duration_since(queued.enqueued_at));
                            input.metrics.input_slot_age.record(started.duration_since(queued.retained_since));
                            hid.send(&event).await?;
                            let completed = now();
                            input.metrics.input_write.record(completed.duration_since(started));
                            input.metrics.input_enqueue_to_write_complete.record(completed.duration_since(queued.enqueued_at));
                            Ok::<(), anyhow::Error>(())
                        };
                        tokio::select! {
                            biased;
                            _ = stop.changed() => return Ok(()),
                            sent = tokio::time::timeout(Duration::from_secs(2), send) => {
                                sent.context("input send deadline exceeded")??;
                            }
                        }
                        if matches!(event, HidEvent::Touch { phase: TouchPhase::AnchorBegin, .. })
                        {
                            // Let the phone sample a new joystick center
                            // before movement. Back-to-back delivery collapsed
                            // the center and first displacement in live testing.
                            // Ordinary taps and ongoing movement do not wait.
                            let deadline = tokio::time::Instant::now() + Duration::from_millis(20);
                            while input.reset_generation.load(Ordering::Acquire) == generation {
                                tokio::select! {
                                    biased;
                                    _ = stop.changed() => return Ok(()),
                                    _ = input.reset.notified() => {},
                                    _ = tokio::time::sleep_until(deadline) => break,
                                }
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
    handoff_at: Instant,
    reset: bool,
}

impl EncodedFrame {
    fn record_handoff(&self, metrics: &Metrics, dequeued_at: Instant) {
        // Starts immediately before bounded send: capacity wait + channel
        // residency only, not network, depacketization, or decoder execution.
        metrics
            .encoded_handoff
            .record(dequeued_at.duration_since(self.handoff_at));
    }
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
    let stream = Arc::new(stream);
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
            packet.record_handoff(&metrics, Instant::now());
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

    let (feedback_tx, feedback_rx) = watch::channel(crate::feedback::Request::default());
    let mut feedback_request = crate::feedback::Request::default();
    let mut feedback_result = None;
    let mut feedback_task = tokio::spawn(crate::feedback::worker(
        stream.clone(),
        stream.info.local_ssrc,
        stream.info.remote_ssrc,
        feedback_rx,
        input_stop.subscribe(),
        shared.metrics.clone(),
        Duration::from_millis(250),
    ));
    let mut depacketizer = HevcDepacketizer::default();
    let mut rate_window = RateWindow::new(Instant::now());
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
                finished = &mut feedback_task => {
                    feedback_result = Some(finished.context("RTCP feedback task failed").and_then(|result| result));
                    break;
                }
                _ = report.tick() => {
                    shared.metrics.report_rates(&mut rate_window);
                    shared.metrics.report();
                    if last_packet.elapsed() > Duration::from_secs(10) { bail!("video packet deadline exceeded"); }
                    if decode.is_finished() { bail!("decoder worker ended"); }
                    feedback_request.sequence = depacketizer.stats().highest_sequence;
                    feedback_tx.send_replace(feedback_request);
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
                        feedback_request.sequence = depacketizer.stats().highest_sequence;
                        feedback_request.keyframe_generation = feedback_request.keyframe_generation.wrapping_add(1);
                        feedback_tx.send_replace(feedback_request);
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
                        bytes, received_at: last_packet, handoff_at: Instant::now(), reset: reset_pending,
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
    let feedback_cleanup = match feedback_result {
        Some(result) => result,
        None => match tokio::time::timeout(Duration::from_secs(1), &mut feedback_task).await {
            Ok(result) => result
                .context("RTCP feedback task failed")
                .and_then(|result| result),
            Err(_) => {
                feedback_task.abort();
                let _ = feedback_task.await;
                Err(anyhow::anyhow!(
                    "RTCP feedback worker shutdown deadline exceeded"
                ))
            }
        },
    };
    let stream_cleanup = tokio::time::timeout(Duration::from_secs(5), session.stop()).await;
    let decoder_result = decode.await.context("decoder thread failed")?;
    shared.metrics.report();
    result?;
    input_cleanup?;
    feedback_cleanup?;
    stream_cleanup.context("stream cleanup deadline exceeded")??;
    decoder_result
}

#[cfg(test)]
mod input_tests {
    use super::*;

    #[tokio::test]
    async fn input_metrics_follow_retained_snapshot_and_successful_write_boundaries() -> Result<()>
    {
        let metrics = Arc::new(Metrics::default());
        let bus = Arc::new(InputBus::new(metrics.clone()));
        let origin = Instant::now();
        let clock_ms = Arc::new(AtomicU64::new(40));
        let motion = HidEvent::Touch {
            phase: TouchPhase::Move,
            report: [0; 58],
        };
        assert!(bus.push_at(motion, origin));
        assert!(bus.push_at(motion, origin + Duration::from_millis(10)));
        assert_eq!(metrics.input_coalesced.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.input_queue_high_water.load(Ordering::Relaxed), 1);

        let (sent, mut received) = mpsc::unbounded_channel();
        let gate = Arc::new(Notify::new());
        let closed = Arc::new(AtomicBool::new(false));
        let sink = RecordedInput {
            sent,
            first_send_gate: Some(gate.clone()),
            closed: closed.clone(),
        };
        let (stop, receiver) = watch::channel(false);
        let clock = clock_ms.clone();
        let worker = input_worker_with_clock(sink, bus, receiver, move || {
            origin + Duration::from_millis(clock.load(Ordering::Relaxed))
        });
        tokio::pin!(worker);
        poll_worker_once(worker.as_mut()).await;
        assert_eq!(received.try_recv()?, motion);
        assert_eq!(metrics.input_queue_age.samples(), 1);
        assert_eq!(metrics.input_queue_age.mean_ms(), 30.0);
        assert_eq!(metrics.input_slot_age.mean_ms(), 40.0);
        assert_eq!(metrics.input_write.samples(), 0);
        assert_eq!(metrics.input_enqueue_to_write_complete.samples(), 0);

        clock_ms.store(47, Ordering::Relaxed);
        gate.notify_one();
        poll_worker_once(worker.as_mut()).await;
        assert_eq!(metrics.input_write.samples(), 1);
        assert_eq!(metrics.input_write.mean_ms(), 7.0);
        assert_eq!(metrics.input_enqueue_to_write_complete.samples(), 1);
        assert_eq!(metrics.input_enqueue_to_write_complete.mean_ms(), 37.0);
        stop.send(true)?;
        worker.await?;
        assert!(closed.load(Ordering::Acquire));
        Ok(())
    }

    #[test]
    fn encoded_handoff_excludes_receive_and_assembly_time() {
        let metrics = Metrics::default();
        let origin = Instant::now();
        let packet = EncodedFrame {
            bytes: Vec::new(),
            received_at: origin,
            handoff_at: origin + Duration::from_millis(80),
            reset: false,
        };
        packet.record_handoff(&metrics, origin + Duration::from_millis(100));
        assert_eq!(metrics.encoded_handoff.samples(), 1);
        assert_eq!(metrics.encoded_handoff.mean_ms(), 20.0);
    }

    struct RecordedInput {
        sent: mpsc::UnboundedSender<HidEvent>,
        first_send_gate: Option<Arc<Notify>>,
        closed: Arc<AtomicBool>,
    }

    impl InputSink for RecordedInput {
        async fn send(&mut self, event: &HidEvent) -> Result<()> {
            self.sent.send(*event)?;
            if let Some(gate) = self.first_send_gate.take() {
                gate.notified().await;
            }
            Ok(())
        }
        async fn close(&mut self) -> Result<()> {
            self.closed.store(true, Ordering::Release);
            Ok(())
        }
    }

    async fn poll_worker_once(worker: std::pin::Pin<&mut impl std::future::Future>) {
        let mut worker = worker;
        std::future::poll_fn(|context| {
            assert!(worker.as_mut().poll(context).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
    }

    async fn cancel_backlog(inflight: bool) -> Result<()> {
        let metrics = Arc::new(Metrics::default());
        let bus = Arc::new(InputBus::new(metrics.clone()));
        let report = iphone_mirror_rs::input::touchscreen_report(true, 100, 200, 1);
        let anchor = HidEvent::Touch {
            phase: TouchPhase::AnchorBegin,
            report,
        };
        let movement = HidEvent::Touch {
            phase: TouchPhase::Move,
            report,
        };
        for _ in 0..20 {
            assert!(bus.push(anchor));
            assert!(bus.push(movement));
        }
        let home = HidEvent::Home { pressed: true };
        let keyboard = HidEvent::Keyboard([0; 39]);
        assert!(bus.push(home));
        assert!(bus.push(keyboard));
        let (sent, mut received) = mpsc::unbounded_channel();
        let closed = Arc::new(AtomicBool::new(false));
        let gate = Arc::new(Notify::new());
        let sink = RecordedInput {
            sent,
            first_send_gate: inflight.then(|| gate.clone()),
            closed: closed.clone(),
        };
        let (stop, receiver) = watch::channel(false);
        let worker = input_worker(sink, bus.clone(), receiver);
        tokio::pin!(worker);
        poll_worker_once(worker.as_mut()).await;
        assert_eq!(received.try_recv()?, anchor);
        assert!(received.try_recv().is_err());

        assert!(bus.cancel_game(2));
        assert!(bus.push(anchor));
        assert!(bus.push(movement));
        poll_worker_once(worker.as_mut()).await;
        if inflight {
            // Reset never drops an in-progress XPC write. Its original future
            // remains pending until completion, then the reset takes priority.
            assert!(received.try_recv().is_err());
            gate.notify_one();
            poll_worker_once(worker.as_mut()).await;
        }
        assert_eq!(
            received.try_recv()?,
            HidEvent::ReleaseTouches { timestamp: 2 }
        );
        assert_eq!(received.try_recv()?, home);
        assert_eq!(received.try_recv()?, keyboard);
        assert_eq!(received.try_recv()?, anchor);
        // Stale reset notifications must not bypass the new anchor's dwell.
        assert!(received.try_recv().is_err());
        stop.send(true)?;
        worker.await?;
        assert!(closed.load(Ordering::Acquire));
        assert!(received.recv().await.is_none());
        // Discarded queued snapshots are not dispatch samples. Both cases send
        // the original anchor, reset, Home, keyboard, and the new anchor only.
        assert_eq!(metrics.input_queue_age.samples(), 5);
        assert_eq!(metrics.input_write.samples(), 5);
        assert_eq!(metrics.input_queue_high_water.load(Ordering::Relaxed), 42);
        Ok(())
    }

    #[tokio::test]
    async fn game_cancel_discards_anchor_backlog_and_resets_before_future_input() -> Result<()> {
        cancel_backlog(false).await
    }

    #[tokio::test]
    async fn game_cancel_finishes_inflight_write_before_reset() -> Result<()> {
        cancel_backlog(true).await
    }

    struct DispatchInput {
        sent: mpsc::UnboundedSender<Instant>,
        closed: Arc<AtomicBool>,
    }

    impl InputSink for DispatchInput {
        async fn send(&mut self, _: &HidEvent) -> Result<()> {
            self.sent.send(Instant::now())?;
            Ok(())
        }
        async fn close(&mut self) -> Result<()> {
            self.closed.store(true, Ordering::Release);
            Ok(())
        }
    }

    async fn queued_dispatch(first_phase: TouchPhase, cancel_during_dwell: bool) -> Result<()> {
        let bus = Arc::new(InputBus::new(Arc::new(Metrics::default())));
        let report = iphone_mirror_rs::input::touchscreen_report(true, 100, 200, 1);
        for phase in [first_phase, TouchPhase::Move] {
            assert!(bus.push(HidEvent::Touch { phase, report }));
        }
        let (sent, mut received) = mpsc::unbounded_channel();
        let closed = Arc::new(AtomicBool::new(false));
        let sink = DispatchInput {
            sent,
            closed: closed.clone(),
        };
        let (stop, receiver) = watch::channel(false);
        let worker = tokio::spawn(input_worker(sink, bus, receiver));
        let first = tokio::time::timeout(Duration::from_secs(1), received.recv())
            .await?
            .context("Begin must be sent")?;
        if first_phase == TouchPhase::Begin {
            // Both ready sends are polled in the same worker turn. A dwell
            // would leave the second event absent until its timer completed.
            assert!(received.try_recv().is_ok());
        } else if !cancel_during_dwell {
            let second = tokio::time::timeout(Duration::from_secs(1), received.recv())
                .await?
                .context("Move must be sent")?;
            assert!(second.duration_since(first) >= Duration::from_millis(20));
        }
        stop.send(true)?;
        tokio::time::timeout(Duration::from_millis(200), worker).await???;
        assert!(closed.load(Ordering::Acquire));
        assert!(received.recv().await.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn anchor_dispatch_delays_first_move() -> Result<()> {
        queued_dispatch(TouchPhase::AnchorBegin, false).await
    }

    #[tokio::test]
    async fn anchor_dispatch_cancellation_skips_move_and_runs_cleanup() -> Result<()> {
        queued_dispatch(TouchPhase::AnchorBegin, true).await
    }

    #[tokio::test]
    async fn ordinary_begin_dispatch_has_no_dwell() -> Result<()> {
        queued_dispatch(TouchPhase::Begin, false).await
    }

    #[test]
    fn overflowing_transitions_fail_session_instead_of_dropping_release() {
        let bus = InputBus::new(Arc::new(Metrics::default()));
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
        let metrics = Arc::new(Metrics::default());
        let bus = Arc::new(InputBus::new(metrics.clone()));
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
        assert_eq!(metrics.input_queue_age.samples(), 1);
        assert_eq!(metrics.input_write.samples(), 0);
        assert_eq!(metrics.input_enqueue_to_write_complete.samples(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn input_write_failure_still_runs_cleanup() -> Result<()> {
        let metrics = Arc::new(Metrics::default());
        let bus = Arc::new(InputBus::new(metrics.clone()));
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
        assert_eq!(metrics.input_queue_age.samples(), 1);
        assert_eq!(metrics.input_write.samples(), 0);
        assert_eq!(metrics.input_enqueue_to_write_complete.samples(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn input_overflow_still_runs_cleanup() -> Result<()> {
        let bus = Arc::new(InputBus::new(Arc::new(Metrics::default())));
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
