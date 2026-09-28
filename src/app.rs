use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use iphone_mirror_rs::input::{
    HidEvent, InputState, TouchPhase, TouchSample, normalized_position, wheel_gesture,
};
use iphone_mirror_rs::metrics::Metrics;
use iphone_mirror_rs::video::{DecodedFrame, LatestFrame, Renderer};
use tokio::sync::watch;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::session::InputBus;

#[derive(Debug)]
pub enum AppEvent {
    Connected,
    FrameReady,
    Finished(bool),
}

pub struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    frame: Option<DecodedFrame>,
    latest: Arc<LatestFrame>,
    input: Arc<InputBus>,
    metrics: Arc<Metrics>,
    stop: watch::Sender<bool>,
    controls: InputState,
    physical_keys: [bool; 240],
    spotlight: bool,
    mouse_down: bool,
    pointer: PhysicalPosition<f64>,
    wheel: Option<([TouchSample; 10], usize, Instant)>,
    pending_scroll: f64,
    clock: Instant,
    connected: bool,
    focused: bool,
    pub failed: bool,
}

impl App {
    pub fn new(
        latest: Arc<LatestFrame>,
        input: Arc<InputBus>,
        metrics: Arc<Metrics>,
        stop: watch::Sender<bool>,
    ) -> Self {
        Self {
            window: None,
            renderer: None,
            frame: None,
            latest,
            input,
            metrics,
            stop,
            controls: InputState::default(),
            physical_keys: [false; 240],
            spotlight: false,
            mouse_down: false,
            pointer: PhysicalPosition::new(0.0, 0.0),
            wheel: None,
            pending_scroll: 0.0,
            clock: Instant::now(),
            connected: false,
            focused: false,
            failed: false,
        }
    }

    fn timestamp(&self) -> u64 {
        self.clock.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
    }

    fn send(&mut self, event: Option<HidEvent>) {
        if let Some(event) = event
            && !self.input.push(event)
        {
            self.failed = true;
            let _ = self.stop.send(true);
        }
    }

    fn release(&mut self) {
        self.wheel = None;
        self.pending_scroll = 0.0;
        self.mouse_down = false;
        self.physical_keys.fill(false);
        self.spotlight = false;
        for event in self.controls.release_all(self.timestamp()) {
            self.send(event);
        }
    }

    fn position(&self, clamp: bool) -> Option<(f64, f64)> {
        let window = self.window.as_ref()?;
        let frame = self.frame.as_ref()?;
        let size = window.inner_size();
        let scale = (f64::from(size.width) / f64::from(frame.width))
            .min(f64::from(size.height) / f64::from(frame.height));
        if scale <= 0.0 {
            return None;
        }
        let width = f64::from(frame.width) * scale;
        let height = f64::from(frame.height) * scale;
        let x = (self.pointer.x - (f64::from(size.width) - width) / 2.0) / width;
        let y = (self.pointer.y - (f64::from(size.height) - height) / 2.0) / height;
        if !clamp && (!(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y)) {
            return None;
        }
        Some((x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)))
    }

    fn start_scroll(&mut self) {
        if self.mouse_down || !self.focused || !self.connected {
            self.pending_scroll = 0.0;
            return;
        }
        if self.wheel.is_some() || self.pending_scroll == 0.0 {
            return;
        }
        if let Some((x, y)) = self.position(false)
            && let Some(samples) = wheel_gesture(x, y, self.pending_scroll, 0)
        {
            self.wheel = Some((samples, 0, Instant::now()));
        }
        self.pending_scroll = 0.0;
    }

    fn key(&mut self, code: KeyCode, pressed: bool) {
        let timestamp = self.timestamp();
        if code == KeyCode::F1 {
            let event = self.controls.home(pressed);
            self.send(event);
        } else if code == KeyCode::F2 {
            // Spotlight is the standard iOS Command+Space shortcut.
            self.spotlight = pressed;
            for usage in if pressed { [227, 44] } else { [44, 227] } {
                let held = pressed || self.physical_keys[usage as usize];
                let event = self.controls.set_key(usage, held, timestamp);
                self.send(event);
            }
        } else if let Some(usage) = hid_usage(code) {
            self.physical_keys[usage as usize] = pressed;
            let held = pressed || self.spotlight && matches!(usage, 227 | 44);
            let event = self.controls.set_key(usage, held, timestamp);
            self.send(event);
        }
    }

    fn cancel_touch(&mut self) {
        self.wheel = None;
        self.pending_scroll = 0.0;
        self.mouse_down = false;
        let event = self.controls.touch_end(self.timestamp());
        self.send(event);
    }

    fn mouse_button(&mut self, pressed: bool, position: Option<(u16, u16)>) {
        // A wheel gesture owns the same single contact as a drag. Release it
        // before beginning the user's new contact, including clicks in margins.
        self.cancel_touch();
        if pressed && let Some((x, y)) = position {
            self.mouse_down = true;
            let event = self.controls.touch_begin(x, y, self.timestamp());
            self.send(event);
        }
    }

    fn advance_scroll(&mut self, now: Instant) {
        if !self.focused || !self.connected {
            if self.wheel.is_some() {
                self.cancel_touch();
            }
            return;
        }
        if let Some((samples, index, next)) = self.wheel.take() {
            if now >= next {
                let sample = samples[index];
                let timestamp = self.timestamp();
                let event = match sample.phase {
                    TouchPhase::Begin => self.controls.touch_begin(sample.x, sample.y, timestamp),
                    TouchPhase::Move => self.controls.touch_move(sample.x, sample.y, timestamp),
                    TouchPhase::End => self.controls.touch_end(timestamp),
                };
                self.send(event);
                if index + 1 < samples.len() {
                    self.wheel = Some((samples, index + 1, now + Duration::from_millis(15)));
                } else {
                    self.start_scroll();
                }
            } else {
                self.wheel = Some((samples, index, next));
            }
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let result = (|| -> anyhow::Result<_> {
            let window = Arc::new(
                event_loop.create_window(
                    Window::default_attributes()
                        .with_title("iPhone Mirror · Connecting")
                        .with_inner_size(LogicalSize::new(400.0, 870.0)),
                )?,
            );
            let renderer = pollster::block_on(Renderer::new(window.clone()))?;
            Ok((window, renderer))
        })();
        match result {
            Ok((window, renderer)) => {
                self.window = Some(window);
                self.renderer = Some(renderer);
            }
            Err(error) => {
                tracing::error!(%error, "GPU window initialization failed");
                self.failed = true;
                let _ = self.stop.send(true);
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Connected => {
                self.connected = true;
                if let Some(window) = &self.window {
                    window.set_title("iPhone Mirror");
                }
            }
            AppEvent::FrameReady => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            AppEvent::Finished(failed) => {
                self.failed |= failed;
                self.release();
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.release();
                let _ = self.stop.send(true);
                event_loop.exit();
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                if !focused {
                    self.release();
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                let fresh = self.latest.take();
                let changed = fresh.is_some();
                if fresh.is_some() {
                    self.frame = fresh;
                }
                if let (Some(renderer), Some(frame)) = (&mut self.renderer, &self.frame) {
                    match renderer.render(frame) {
                        Ok(()) if changed => {
                            self.metrics.submitted.fetch_add(1, Ordering::Relaxed);
                            self.metrics
                                .receive_to_submit
                                .record(frame.received_at.elapsed());
                            self.metrics.record_source_stamp(frame);
                        }
                        Ok(()) => {}
                        Err(error) => {
                            tracing::error!(%error, "GPU frame submission failed");
                            self.failed = true;
                            let _ = self.stop.send(true);
                            event_loop.exit();
                        }
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = position;
                if self.focused
                    && self.connected
                    && self.wheel.is_none()
                    && let Some((x, y)) = self
                        .position(true)
                        .and_then(|(x, y)| normalized_position(x, y, 0))
                {
                    let event = self.controls.touch_move(x, y, self.timestamp());
                    self.send(event);
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.cancel_touch();
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } if self.focused && self.connected => {
                let position = self
                    .position(false)
                    .and_then(|(x, y)| normalized_position(x, y, 0));
                self.mouse_button(state == ElementState::Pressed, position);
            }
            WindowEvent::MouseWheel { delta, .. } if self.focused && self.connected => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => f64::from(y),
                    MouseScrollDelta::PixelDelta(p) => p.y / 50.0,
                };
                self.pending_scroll = (self.pending_scroll + lines).clamp(-4.0, 4.0);
                self.start_scroll();
            }
            WindowEvent::KeyboardInput { event, .. }
                if self.focused && self.connected && !event.repeat =>
            {
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.key(code, event.state.is_pressed());
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.advance_scroll(Instant::now());
        event_loop.set_control_flow(
            self.wheel
                .as_ref()
                .map_or(ControlFlow::Wait, |(_, _, at)| ControlFlow::WaitUntil(*at)),
        );
    }
}

fn hid_usage(code: KeyCode) -> Option<u8> {
    use KeyCode::*;
    let letters = [
        KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO,
        KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
    ];
    if let Some(i) = letters.iter().position(|&key| key == code) {
        return Some(4 + i as u8);
    }
    Some(match code {
        Digit1 => 30,
        Digit2 => 31,
        Digit3 => 32,
        Digit4 => 33,
        Digit5 => 34,
        Digit6 => 35,
        Digit7 => 36,
        Digit8 => 37,
        Digit9 => 38,
        Digit0 => 39,
        Enter => 40,
        NumpadEnter => 88,
        Escape => 41,
        Backspace => 42,
        Tab => 43,
        Space => 44,
        Minus => 45,
        Equal => 46,
        BracketLeft => 47,
        BracketRight => 48,
        Backslash => 49,
        Semicolon => 51,
        Quote => 52,
        Backquote => 53,
        Comma => 54,
        Period => 55,
        Slash => 56,
        CapsLock => 57,
        Insert => 73,
        Home => 74,
        PageUp => 75,
        Delete => 76,
        End => 77,
        PageDown => 78,
        ArrowRight => 79,
        ArrowLeft => 80,
        ArrowDown => 81,
        ArrowUp => 82,
        ControlLeft => 224,
        ShiftLeft => 225,
        AltLeft => 226,
        SuperLeft => 227,
        ControlRight => 228,
        ShiftRight => 229,
        AltRight => 230,
        SuperRight => 231,
        _ => return None,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    fn app() -> App {
        let (stop, _receiver) = watch::channel(false);
        let mut app = App::new(
            Arc::new(LatestFrame::new()),
            Arc::new(InputBus::new()),
            Arc::new(Metrics::default()),
            stop,
        );
        app.focused = true;
        app.connected = true;
        app
    }
    fn events(app: &App) -> Vec<HidEvent> {
        let mut events = Vec::new();
        while let Some(event) = app.input.pop().unwrap() {
            events.push(event);
        }
        events
    }
    fn begin_wheel(app: &mut App) -> Instant {
        let now = Instant::now();
        app.wheel = Some((wheel_gesture(0.5, 0.5, -1.0, 0).unwrap(), 0, now));
        app.advance_scroll(now);
        now
    }
    #[test]
    fn hardware_keys_have_stable_hid_usages() {
        assert_eq!(hid_usage(KeyCode::KeyA), Some(4));
        assert_eq!(hid_usage(KeyCode::KeyZ), Some(29));
        assert_eq!(hid_usage(KeyCode::ShiftLeft), Some(225));
        assert_eq!(hid_usage(KeyCode::SuperRight), Some(231));
        assert_eq!(hid_usage(KeyCode::F1), None);
        assert_eq!(hid_usage(KeyCode::NumpadEnter), Some(88));
    }
    #[test]
    fn mouse_click_interrupts_wheel_with_release_before_new_begin() {
        let mut app = app();
        begin_wheel(&mut app);
        app.mouse_button(true, Some((1000, 2000)));
        app.mouse_button(false, None);
        let phases: Vec<_> = events(&app)
            .into_iter()
            .filter_map(|event| match event {
                HidEvent::Touch { phase, .. } => Some(phase),
                _ => None,
            })
            .collect();
        assert_eq!(
            phases,
            [
                TouchPhase::Begin,
                TouchPhase::End,
                TouchPhase::Begin,
                TouchPhase::End
            ]
        );
        assert!(app.wheel.is_none());
        assert!(!app.mouse_down);
    }
    #[test]
    fn held_mouse_drag_does_not_start_wheel_or_leave_pending_scroll() {
        let mut app = app();
        app.mouse_button(true, Some((1000, 2000)));
        app.pending_scroll = 4.0;
        app.start_scroll();
        assert!(app.wheel.is_none());
        assert_eq!(app.pending_scroll, 0.0);
        assert_eq!(events(&app).len(), 1);
    }
    #[test]
    fn focus_loss_releases_touch_keyboard_and_home_and_cancels_scheduled_moves() {
        let mut app = app();
        let now = begin_wheel(&mut app);
        app.key(KeyCode::KeyA, true);
        app.key(KeyCode::F1, true);
        events(&app);
        app.focused = false;
        app.release();
        app.advance_scroll(now + Duration::from_secs(1));
        let events = events(&app);
        assert_eq!(events.len(), 3);
        assert!(matches!(
            events[0],
            HidEvent::Touch {
                phase: TouchPhase::End,
                ..
            }
        ));
        let HidEvent::Keyboard(report) = events[1] else {
            panic!("keyboard release");
        };
        assert!(report[1..31].iter().all(|&byte| byte == 0));
        assert_eq!(events[2], HidEvent::Home { pressed: false });
        assert!(app.wheel.is_none());
    }
    #[test]
    fn spotlight_does_not_release_keys_held_by_physical_keyboard() {
        let mut app = app();
        app.key(KeyCode::SuperLeft, true);
        app.key(KeyCode::Space, true);
        events(&app);
        app.key(KeyCode::F2, true);
        app.key(KeyCode::F2, false);
        assert!(events(&app).is_empty());
        app.key(KeyCode::Space, false);
        let HidEvent::Keyboard(report) = events(&app)[0] else {
            panic!("keyboard");
        };
        assert_eq!(report[1 + 44 / 8] & (1 << (44 % 8)), 0);
        assert_ne!(report[1 + 227 / 8] & (1 << (227 % 8)), 0);
    }
    #[test]
    fn physical_release_preserves_spotlight_chord_until_shortcut_is_released() {
        let mut app = app();
        app.key(KeyCode::F2, true);
        events(&app);
        app.key(KeyCode::SuperLeft, true);
        app.key(KeyCode::SuperLeft, false);
        assert!(events(&app).is_empty());
        app.key(KeyCode::F2, false);
        let reports = events(&app);
        let Some(HidEvent::Keyboard(report)) = reports.last() else {
            panic!("keyboard release");
        };
        assert!(report[1..31].iter().all(|&byte| byte == 0));
    }
    #[test]
    fn wheel_ends_at_final_contact_and_leaves_no_stuck_touch() {
        let mut app = app();
        let start = begin_wheel(&mut app);
        for step in 1..10 {
            app.advance_scroll(start + Duration::from_millis(step * 15));
        }
        let events = events(&app);
        let Some(HidEvent::Touch {
            phase: TouchPhase::End,
            report: release,
        }) = events.last()
        else {
            panic!("touch release");
        };
        let HidEvent::Touch {
            phase: TouchPhase::Move,
            report: contact,
        } = &events[events.len() - 2]
        else {
            panic!("touch move");
        };
        assert_eq!(&release[4..8], &contact[4..8]);
        assert!(app.controls.touch_end(0).is_none());
        assert!(app.wheel.is_none());
    }
}
