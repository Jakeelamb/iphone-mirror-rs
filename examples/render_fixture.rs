//! Offline surface smoke test: `cargo run --example render_fixture`.
//! Add `--hardware` to exercise hardware decode and NV12 presentation.
//! Displays only synthetic testsrc2 content and never opens a device connection.
use anyhow::{Context, Result};
use iphone_mirror_rs::video::{DecodeMode, DecodedFrame, Decoder, Renderer};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

struct Fixture {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    decoder: Decoder,
    packets: Vec<&'static [u8]>,
    frame: Option<DecodedFrame>,
    next_frame: Instant,
    count: usize,
    error: Option<anyhow::Error>,
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
        self.renderer = Some(pollster::block_on(Renderer::new(window.clone()))?);
        self.window = Some(window);
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        // Repeat the first picture for deterministic screenshot comparison.
        self.decoder
            .decode(self.packets[0], Instant::now(), |frame| {
                self.frame = Some(frame);
            })?;
        self.count += 1;
        if let (Some(renderer), Some(frame)) = (&mut self.renderer, &self.frame) {
            renderer.render(frame)?;
        }
        Ok(())
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        self.error = Some(error);
        event_loop.exit();
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
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.next() {
                    self.fail(event_loop, error);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if Instant::now() >= self.next_frame {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            self.next_frame = Instant::now() + Duration::from_millis(33);
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("iphone_mirror_rs=info,render_fixture=info")
        .init();
    let data = include_bytes!("../src/video/fixtures/motion-256x384.hevc");
    let mut boundaries: Vec<_> = data
        .windows(6)
        .enumerate()
        .filter_map(|(index, bytes)| {
            (bytes[..4] == [0, 0, 0, 1] && bytes[4] >> 1 == 35).then_some(index)
        })
        .collect();
    boundaries.push(data.len());
    let packets = boundaries
        .windows(2)
        .map(|range| &data[range[0]..range[1]])
        .collect();
    let mode = if std::env::args().any(|argument| argument == "--hardware") {
        DecodeMode::Auto
    } else {
        DecodeMode::Software
    };
    let mut app = Fixture {
        window: None,
        renderer: None,
        decoder: Decoder::new(mode)?,
        packets,
        frame: None,
        next_frame: Instant::now(),
        count: 0,
        error: None,
    };
    EventLoop::new()?
        .run_app(&mut app)
        .context("fixture event loop")?;
    if let Some(error) = app.error {
        return Err(error);
    }
    tracing::info!(frames = app.count, "synthetic fixture closed");
    Ok(())
}
