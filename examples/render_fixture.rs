//! Offline surface smoke test: `cargo run --example render_fixture`.
//! Add `--hardware` to exercise hardware decode and NV12 presentation.
//! Add `--rotation 0|90|180|270` to check clockwise GPU rotation.
//! Displays only synthetic testsrc2 content and never opens a device connection.
use anyhow::{Context, Result, bail};
use iphone_mirror_rs::video::{
    DecodeMode, DecodedFrame, Decoder, Renderer, ViewerLayout, displayed_size,
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
    picture: &'static [u8],
    frame: Option<DecodedFrame>,
    next_frame: Instant,
    count: usize,
    error: Option<anyhow::Error>,
    home_hovered: bool,
    home_armed: bool,
    rotation: u16,
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
        let mut renderer = pollster::block_on(Renderer::new(window.clone()))?;
        renderer.set_rotation(self.rotation);
        self.renderer = Some(renderer);
        self.window = Some(window);
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        // Repeat the first picture for deterministic screenshot comparison.
        self.decoder.decode(self.picture, Instant::now(), |frame| {
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

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("iphone_mirror_rs=info,render_fixture=info")
        .init();
    let data = include_bytes!("../src/video/fixtures/motion-256x384.hevc");
    let picture = first_picture(data)?;
    let mut mode = DecodeMode::Software;
    let mut rotation = 0;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--hardware" => mode = DecodeMode::Auto,
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
            _ => bail!("unknown argument; use --hardware or --rotation 0|90|180|270"),
        }
    }
    let mut app = Fixture {
        window: None,
        renderer: None,
        decoder: Decoder::new(mode)?,
        picture,
        frame: None,
        next_frame: Instant::now(),
        count: 0,
        error: None,
        home_hovered: false,
        home_armed: false,
        rotation,
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

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
