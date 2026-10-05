//! Offline surface smoke test: `cargo run --example render_fixture`.
//! Add `--hardware` to exercise hardware decode and NV12 presentation.
//! Add `--rotation 0|90|180|270` to check clockwise GPU rotation.
//! Add --animate --fps 60 --frames 180 for a bounded moving-fixture run.
//! Accepts the viewer's presentation experiment flags. Never opens a device connection.
use anyhow::{Context, Result, bail};
use iphone_mirror_rs::metrics::Histogram;
use iphone_mirror_rs::video::{
    DecodeMode, DecodedFrame, Decoder, PresentationOptions, RenderSample, Renderer, ViewerLayout,
    displayed_size,
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

struct Fixture {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    decoder: Decoder,
    pictures: Vec<&'static [u8]>,
    frame: Option<DecodedFrame>,
    next_frame: Instant,
    count: usize,
    error: Option<anyhow::Error>,
    home_hovered: bool,
    home_armed: bool,
    rotation: u16,
    presentation: PresentationOptions,
    frame_interval: Duration,
    frame_limit: Option<usize>,
    pending: bool,
    replaced: usize,
    submitted: usize,
    acquire: Histogram,
    render: Histogram,
}

impl Fixture {
    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("Rust mirror synthetic fixture")
                    .with_inner_size(LogicalSize::new(600, 700)),
            )?,
        );
        let mut renderer = pollster::block_on(Renderer::new(window.clone(), self.presentation))?;
        renderer.set_rotation(self.rotation);
        self.renderer = Some(renderer);
        self.window = Some(window);
        Ok(())
    }

    fn source_finished(&self, now: Instant) -> bool {
        now >= self.next_frame && self.frame_limit.is_some_and(|limit| self.count >= limit)
    }

    /// The main-loop timer advances the source even when redraws are throttled.
    /// A blocked main thread still delays this timer; this is not a producer thread.
    fn source_tick(&mut self, now: Instant) -> Result<bool> {
        if now < self.next_frame || self.source_finished(now) {
            return Ok(false);
        }
        let picture = self.pictures[self.count % self.pictures.len()];
        self.decoder.decode(picture, now, |frame| {
            self.replaced += usize::from(self.pending);
            self.frame = Some(frame);
            self.pending = true;
        })?;
        self.count += 1;
        // Preserve the target phase without replaying a burst after a stall.
        self.next_frame += self.frame_interval;
        if self.next_frame <= now {
            self.next_frame = now + self.frame_interval;
        }
        Ok(true)
    }

    fn record_render(&mut self, sample: RenderSample) {
        self.acquire.record(sample.surface_acquire);
        self.render.record(sample.total);
        if sample.submitted_at.is_some() && self.pending {
            self.pending = false;
            self.submitted += 1;
        }
    }

    fn render_current(&mut self) -> Result<()> {
        if let (Some(renderer), Some(frame)) = (&mut self.renderer, &self.frame) {
            let sample = renderer.render(frame)?;
            self.record_render(sample);
        }
        Ok(())
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        self.error = Some(error);
        event_loop.exit();
    }

    fn button_feedback(&mut self) {
        if let Some(renderer) = &mut self.renderer
            && renderer.set_home_state(self.home_hovered, self.home_armed && self.home_hovered)
            && let Some(window) = &self.window
        {
            window.request_redraw();
        }
    }
}

impl ApplicationHandler for Fixture {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none()
            && let Err(error) = self.initialize(event_loop)
        {
            self.fail(event_loop, error);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.home_hovered = false;
                self.home_armed = false;
                self.button_feedback();
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let (Some(window), Some(frame)) = (&self.window, &self.frame) {
                    let size = window.inner_size();
                    let (width, height) = displayed_size(frame.width, frame.height, self.rotation);
                    self.home_hovered = ViewerLayout::new(
                        size.width,
                        size.height,
                        width,
                        height,
                        window.scale_factor(),
                    )
                    .home_contains(position.x, position.y);
                    self.button_feedback();
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if !state.is_pressed() && self.home_armed && self.home_hovered {
                    tracing::info!("synthetic Home activated; no device connected");
                }
                self.home_armed = state.is_pressed() && self.home_hovered;
                self.button_feedback();
            }
            WindowEvent::CursorLeft { .. } | WindowEvent::Focused(false) => {
                self.home_hovered = false;
                self.home_armed = false;
                self.button_feedback();
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.render_current() {
                    self.fail(event_loop, error);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        // Allow the last source frame one interval to render, then exit even if
        // occlusion or a compositor callback prevents any further redraw.
        if self.source_finished(now) {
            event_loop.exit();
            return;
        }
        match self.source_tick(now) {
            Ok(true) => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            Ok(false) => {}
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }
}

// The fixture has four-byte start codes and explicit access-unit delimiters.
// Select only the first complete picture; later pictures are not replayed.
fn first_picture(data: &[u8]) -> Result<&[u8]> {
    let mut boundaries = data.windows(6).enumerate().filter_map(|(index, bytes)| {
        (bytes[..4] == [0, 0, 0, 1] && bytes[4] >> 1 == 35).then_some(index)
    });
    let start = boundaries
        .next()
        .context("fixture has no access-unit delimiter")?;
    let end = boundaries.next().unwrap_or(data.len());
    Ok(&data[start..end])
}

fn pictures(data: &[u8], animate: bool) -> Result<Vec<&[u8]>> {
    if !animate {
        return Ok(vec![first_picture(data)?]);
    }
    let mut boundaries: Vec<usize> = data
        .windows(6)
        .enumerate()
        .filter_map(|(index, bytes)| {
            (bytes[..4] == [0, 0, 0, 1] && bytes[4] >> 1 == 35).then_some(index)
        })
        .collect();
    if boundaries.is_empty() {
        bail!("fixture has no access-unit delimiter");
    }
    boundaries.push(data.len());
    Ok(boundaries
        .windows(2)
        .map(|range| &data[range[0]..range[1]])
        .collect())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("iphone_mirror_rs=info,render_fixture=info")
        .init();
    let data = include_bytes!("../src/video/fixtures/motion-256x384.hevc");
    let mut mode = DecodeMode::Software;
    let mut rotation = 0;
    let mut presentation = PresentationOptions::default();
    let mut animate = false;
    let mut fps = 30_u32;
    let mut frame_limit = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--hardware" => mode = DecodeMode::Auto,
            "--animate" => animate = true,
            "--renderer-gpu" | "--present-mode" | "--frame-latency" | "--pre-present-notify" => {
                let value = arguments
                    .next()
                    .with_context(|| format!("{argument} requires a value"))?;
                presentation.set_option(&argument, &value)?;
            }
            "--fps" => {
                fps = arguments
                    .next()
                    .context("--fps requires a value")?
                    .parse()?;
                if !(1..=240).contains(&fps) {
                    bail!("--fps must be within 1..240");
                }
            }
            "--frames" => {
                let frames = arguments
                    .next()
                    .context("--frames requires a value")?
                    .parse::<usize>()?;
                if !(1..=1_000_000).contains(&frames) {
                    bail!("--frames must be within 1..1000000");
                }
                frame_limit = Some(frames);
            }
            "--rotation" => {
                rotation = arguments
                    .next()
                    .context("--rotation requires degrees")?
                    .parse::<u16>()
                    .context("invalid rotation")?;
                if !matches!(rotation, 0 | 90 | 180 | 270) {
                    bail!("rotation must be 0, 90, 180 or 270");
                }
            }
            _ => bail!("unknown fixture argument; see docs/development.md"),
        }
    }
    let mut app = Fixture {
        window: None,
        renderer: None,
        decoder: Decoder::new(mode)?,
        pictures: pictures(data, animate)?,
        frame: None,
        next_frame: Instant::now(),
        count: 0,
        error: None,
        home_hovered: false,
        home_armed: false,
        rotation,
        presentation,
        frame_interval: Duration::from_secs_f64(1.0 / f64::from(fps)),
        frame_limit,
        pending: false,
        replaced: 0,
        submitted: 0,
        acquire: Histogram::default(),
        render: Histogram::default(),
    };
    EventLoop::new()?
        .run_app(&mut app)
        .context("fixture event loop")?;
    if let Some(error) = app.error {
        return Err(error);
    }
    tracing::info!(
        frames = app.count,
        submitted = app.submitted,
        replaced = app.replaced,
        pending = app.pending,
        acquire_p95_ms = app.acquire.percentile_ms(95),
        render_mean_ms = app.render.mean_ms(),
        "synthetic fixture closed; host timings are not scanout measurements"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Result<Fixture> {
        Ok(Fixture {
            window: None,
            renderer: None,
            decoder: Decoder::new(DecodeMode::Software)?,
            pictures: pictures(
                include_bytes!("../src/video/fixtures/motion-256x384.hevc"),
                true,
            )?,
            frame: None,
            next_frame: Instant::now(),
            count: 0,
            error: None,
            home_hovered: false,
            home_armed: false,
            rotation: 0,
            presentation: PresentationOptions::default(),
            frame_interval: Duration::from_secs_f64(1.0 / 60.0),
            frame_limit: Some(3),
            pending: false,
            replaced: 0,
            submitted: 0,
            acquire: Histogram::default(),
            render: Histogram::default(),
        })
    }

    #[test]
    fn source_ticks_are_independent_of_ui_redraws_and_finish_without_submission() -> Result<()> {
        let mut app = fixture()?;
        let start = app.next_frame;
        assert!(app.source_tick(start)?);
        let received_at = app.frame.as_ref().context("decoded frame")?.received_at;
        for _ in 0..20 {
            // Exercise the same path as RedrawRequested without opening a GPU surface.
            app.render_current()?;
            assert!(!app.source_tick(start)?);
        }
        assert_eq!(app.count, 1);
        assert_eq!(
            app.frame.as_ref().context("retained frame")?.received_at,
            received_at
        );
        assert!(app.source_tick(start + app.frame_interval)?);
        assert!(app.source_tick(start + app.frame_interval * 2)?);
        assert_eq!((app.count, app.replaced, app.submitted), (3, 2, 0));
        assert!(!app.source_finished(start + app.frame_interval * 2));
        assert!(app.source_finished(start + app.frame_interval * 3));
        assert!(!app.source_tick(start + app.frame_interval * 3)?);
        assert_eq!(app.count, 3);
        Ok(())
    }

    #[test]
    fn submissions_count_distinct_frames_and_timeout_keeps_latest_pending() -> Result<()> {
        let mut app = fixture()?;
        let now = app.next_frame;
        let sample = |submitted_at| RenderSample {
            submitted_at,
            upload: Duration::ZERO,
            surface_acquire: Duration::ZERO,
            total: Duration::ZERO,
        };
        app.source_tick(now)?;
        app.record_render(sample(None));
        assert!(app.pending);
        app.source_tick(now + app.frame_interval)?;
        assert_eq!(app.replaced, 1);
        app.record_render(sample(Some(now)));
        assert!(!app.pending);
        app.record_render(sample(Some(now)));
        assert_eq!(app.submitted, 1);
        app.source_tick(now + app.frame_interval * 2)?;
        app.record_render(sample(Some(now)));
        assert_eq!((app.count, app.submitted, app.replaced), (3, 2, 1));
        Ok(())
    }

    #[test]
    fn delayed_timer_skips_burst_without_depending_on_redraw() -> Result<()> {
        let mut app = fixture()?;
        let late = app.next_frame + app.frame_interval * 10;
        assert!(app.source_tick(late)?);
        assert_eq!(app.count, 1);
        assert_eq!(app.next_frame, late + app.frame_interval);
        assert!(!app.source_tick(late)?);
        Ok(())
    }

    #[test]
    fn selected_picture_decodes_without_waiting_for_another_packet() -> Result<()> {
        let data = include_bytes!("../src/video/fixtures/motion-256x384.hevc");
        let picture = first_picture(data)?;
        assert!(picture.len() < data.len());
        let mut decoder = Decoder::new(DecodeMode::Software)?;
        let mut count = 0;
        decoder.decode(picture, Instant::now(), |frame| {
            assert_eq!((frame.width, frame.height), (256, 384));
            count += 1;
        })?;
        assert_eq!(count, 1);
        Ok(())
    }

    #[test]
    fn missing_delimiter_is_reported_without_indexing_an_empty_list() {
        assert!(first_picture(&[]).is_err());
        assert!(first_picture(&[0; 16]).is_err());
        assert!(pictures(&[], true).is_err());
    }

    #[test]
    fn animated_fixture_loops_with_new_output_for_each_complete_picture() -> Result<()> {
        let data = include_bytes!("../src/video/fixtures/motion-256x384.hevc");
        let pictures = pictures(data, true)?;
        assert!(pictures.len() > 1);
        let mut decoder = Decoder::new(DecodeMode::Software)?;
        let mut checksums = Vec::new();
        for picture in pictures.iter().cycle().take(pictures.len() * 2) {
            let mut outputs = 0;
            decoder.decode(picture, Instant::now(), |frame| {
                let mut checksum = 0_u64;
                for y in (0..frame.height).step_by(8) {
                    for x in (0..frame.width).step_by(8) {
                        checksum = checksum
                            .wrapping_mul(31)
                            .wrapping_add(u64::from(frame.luma_at(x, y).unwrap_or(0)));
                    }
                }
                checksums.push(checksum);
                outputs += 1;
            })?;
            assert_eq!(
                outputs, 1,
                "a complete AU must produce one fixture picture immediately"
            );
        }
        assert!(
            checksums[..pictures.len()]
                .windows(2)
                .any(|pair| pair[0] != pair[1])
        );
        assert_eq!(&checksums[..pictures.len()], &checksums[pictures.len()..]);
        Ok(())
    }
}
