use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use iphone_mirror_rs::game::bindings::{self, Input, RuntimeBindings, WheelDirection};
use iphone_mirror_rs::game::{Action, EventBatch, GameState};
use iphone_mirror_rs::input::{
    HidEvent, InputState, TouchPhase, TouchSample, ascii_usage, normalized_position, wheel_gesture,
};
use iphone_mirror_rs::metrics::Metrics;
use iphone_mirror_rs::video::{
    DecodedFrame, LatestFrame, PresentationOptions, RenderSample, Renderer, ViewerLayout,
    displayed_size, visual_rotation,
};
use tokio::sync::watch;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{
    DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, CursorIcon, Window, WindowId};

use crate::session::InputBus;

#[derive(Debug)]
pub enum AppEvent {
    Connected,
    FrameReady,
    OrientationChanged(u32),
    Finished(bool),
}

pub struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    frame: Option<DecodedFrame>,
    frame_pending: bool,
    last_submission: Option<Instant>,
    latest: Arc<LatestFrame>,
    input: Arc<InputBus>,
    metrics: Arc<Metrics>,
    stop: watch::Sender<bool>,
    controls: InputState,
    held_usages: [u8; 240],
    key_chords: [Option<(u8, bool)>; 240],
    spotlight: bool,
    mouse_down: bool,
    home_armed: bool,
    keyboard_home: bool,
    pointer: PhysicalPosition<f64>,
    wheel: Option<([TouchSample; 10], usize, Instant)>,
    pending_scroll: f64,
    clock: Instant,
    connected: bool,
    orientation: u8,
    rotation: u16,
    display_size: Option<(u32, u32)>,
    focused: bool,
    game_bindings: RuntimeBindings,
    game_wheel_deadlines: [Option<u64>; 4],
    pub game: Option<crate::game_ui::GameControls>,
    pub presentation: PresentationOptions,
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
            frame_pending: false,
            last_submission: None,
            latest,
            input,
            metrics,
            stop,
            controls: InputState::default(),
            held_usages: [0; 240],
            key_chords: [None; 240],
            spotlight: false,
            mouse_down: false,
            home_armed: false,
            keyboard_home: false,
            pointer: PhysicalPosition::new(0.0, 0.0),
            wheel: None,
            pending_scroll: 0.0,
            clock: Instant::now(),
            connected: false,
            orientation: 1,
            rotation: 0,
            display_size: None,
            focused: false,
            game_bindings: RuntimeBindings::new(),
            game_wheel_deadlines: [None; 4],
            game: None,
            presentation: PresentationOptions::default(),
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
        self.exit_game();
        self.wheel = None;
        self.pending_scroll = 0.0;
        self.mouse_down = false;
        self.held_usages.fill(0);
        self.key_chords.fill(None);
        self.spotlight = false;
        self.home_armed = false;
        self.keyboard_home = false;
        for event in self.controls.release_all(self.timestamp()) {
            self.send(event);
        }
    }

    fn layout(&self) -> Option<ViewerLayout> {
        let window = self.window.as_ref()?;
        let frame = self.frame.as_ref()?;
        let size = window.inner_size();
        let (width, height) = displayed_size(frame.width, frame.height, self.rotation);
        Some(ViewerLayout::new(
            size.width,
            size.height,
            width,
            height,
            window.scale_factor(),
        ))
    }

    fn set_rotation(&mut self, rotation: u16) {
        if self.rotation != rotation {
            self.exit_game();
            if let Some(game) = &mut self.game {
                game.calibration = None;
            }
            self.update_title();
            // End contacts in the old coordinate space before changing mapping.
            self.cancel_touch();
            self.home_armed = false;
            self.rotation = rotation;
            if let Some(renderer) = &mut self.renderer {
                renderer.set_rotation(rotation);
            }
        }
    }

    fn apply_view(&mut self) {
        let Some(frame) = &self.frame else {
            return;
        };
        let rotation = visual_rotation(self.orientation, frame.width, frame.height);
        let dimensions = displayed_size(frame.width, frame.height, rotation);
        self.set_rotation(rotation);
        if self.display_size != Some(dimensions) {
            self.exit_game();
            self.cancel_touch();
            self.home_armed = false;
            self.display_size = Some(dimensions);
            if let Some(window) = &self.window {
                let dpi = window.scale_factor();
                let size = window.inner_size().to_logical::<f64>(dpi);
                let limit = window.current_monitor().map(|monitor| {
                    let size = monitor.size().to_logical::<f64>(dpi);
                    (size.width * 0.9, size.height * 0.9)
                });
                let (width, height) =
                    fitted_window_size((size.width, size.height), dimensions, limit);
                // Wayland tiling/maximization policy may override this request.
                let _ = window.request_inner_size(LogicalSize::new(width, height));
            }
        }
        self.update_home_visual();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn position(&self, clamp: bool) -> Option<(f64, f64)> {
        self.layout()?
            .screen_position(self.pointer.x, self.pointer.y, clamp)
    }

    fn home_hit(&self) -> bool {
        self.layout()
            .is_some_and(|layout| layout.home_contains(self.pointer.x, self.pointer.y))
    }

    fn update_home_visual(&mut self) {
        let hovered = self.connected && self.focused && self.home_hit();
        let changed = self
            .renderer
            .as_mut()
            .is_some_and(|renderer| renderer.set_home_state(hovered, hovered && self.home_armed));
        if let Some(window) = &self.window {
            window.set_cursor(if hovered {
                CursorIcon::Pointer
            } else {
                CursorIcon::Default
            });
            if changed {
                window.request_redraw();
            }
        }
    }

    fn start_scroll(&mut self) {
        if self.mouse_down || self.home_armed || !self.focused || !self.connected {
            self.pending_scroll = 0.0;
            return;
        }
        if self.wheel.is_some() || self.pending_scroll == 0.0 {
            return;
        }
        if let Some((x, y)) = self.position(false)
            && let Some(samples) = wheel_gesture(x, y, self.pending_scroll, self.rotation)
        {
            self.wheel = Some((samples, 0, Instant::now()));
        }
        self.pending_scroll = 0.0;
    }

    #[cfg(test)]
    fn key(&mut self, code: KeyCode, pressed: bool) {
        self.key_event(code, pressed, None);
    }

    fn update_usage(&mut self, usage: u8, pressed: bool, timestamp: u64) {
        let count = &mut self.held_usages[usage as usize];
        if pressed {
            *count = count.saturating_add(1);
        } else {
            *count = count.saturating_sub(1);
        }
        let held = *count != 0 || self.spotlight && matches!(usage, 227 | 44);
        let event = self.controls.set_key(usage, held, timestamp);
        self.send(event);
    }

    fn key_event(&mut self, code: KeyCode, pressed: bool, character: Option<char>) {
        if self.game_key(code, pressed) {
            return;
        }
        let timestamp = self.timestamp();
        if code == KeyCode::F1 {
            self.keyboard_home = pressed;
            let event = self.controls.home(pressed);
            self.send(event);
        } else if code == KeyCode::F2 {
            // Spotlight is the standard iOS Command+Space shortcut.
            self.spotlight = pressed;
            for usage in if pressed { [227, 44] } else { [44, 227] } {
                let held = pressed || self.held_usages[usage as usize] != 0;
                let event = self.controls.set_key(usage, held, timestamp);
                self.send(event);
            }
        } else if let Some(source) = hid_usage(code) {
            if pressed {
                if self.key_chords[source as usize].is_some() {
                    return;
                }
                // A compositor can provide ':' with a virtual keymap and no
                // physical Shift event. Keep that implicit modifier attached to
                // this key until release, independently of other held keys.
                let chord = character.and_then(ascii_usage).unwrap_or((source, false));
                self.key_chords[source as usize] = Some(chord);
                if chord.1 {
                    self.update_usage(225, true, timestamp);
                }
                self.update_usage(chord.0, true, timestamp);
            } else if let Some(chord) = self.key_chords[source as usize].take() {
                // Release uses the press-time mapping even if the layout or
                // modifiers changed while the physical key was held.
                self.update_usage(chord.0, false, timestamp);
                if chord.1 {
                    self.update_usage(225, false, timestamp);
                }
            }
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

    fn pointer_button(&mut self, pressed: bool, home_hit: bool, position: Option<(u16, u16)>) {
        if pressed {
            self.home_armed = home_hit;
            self.mouse_button(true, if home_hit { None } else { position });
        } else {
            let activate = self.home_armed && home_hit;
            self.home_armed = false;
            self.mouse_button(false, None);
            // A click is a complete Home tap. Do not release a Home key still
            // held by F1, and do not send phone touches for the toolbar button.
            if activate && !self.keyboard_home {
                let event = self.controls.home(true);
                self.send(event);
                let event = self.controls.home(false);
                self.send(event);
            }
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
                    TouchPhase::Begin | TouchPhase::AnchorBegin => {
                        self.controls.touch_begin(sample.x, sample.y, timestamp)
                    }
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

    fn game_active(&self) -> bool {
        self.game.as_ref().is_some_and(|game| game.state.is_some())
    }

    fn update_title(&mut self) {
        let title = self.game.as_ref().map(|game| game.title());
        if let Some(window) = &self.window {
            if let Some(title) = &title {
                window.set_title(title);
            } else {
                window.set_title("iPhone Mirror");
            }
        }
        if let Some(renderer) = &mut self.renderer
            && renderer.set_status(title.as_deref().unwrap_or(""))
            && let Some(window) = &self.window
        {
            window.request_redraw();
        }
    }

    fn send_game(&mut self, batch: EventBatch) {
        self.metrics
            .look_resets
            .fetch_add(u64::from(batch.look_resets), Ordering::Relaxed);
        if batch.clipped_motion {
            self.metrics
                .clipped_mouse_events
                .fetch_add(1, Ordering::Relaxed);
        }
        if let Some(error) = &batch.error {
            tracing::warn!(%error, "game input rejected");
        }
        for event in batch {
            self.send(event);
        }
    }

    fn record_render(&mut self, sample: RenderSample) {
        self.metrics.render_attempts.fetch_add(1, Ordering::Relaxed);
        self.metrics.upload.record(sample.upload);
        self.metrics.surface_acquire.record(sample.surface_acquire);
        self.metrics.render_submit.record(sample.total);
        let Some(submitted_at) = sample.submitted_at else {
            self.metrics
                .surface_timeouts
                .fetch_add(1, Ordering::Relaxed);
            // A later UI redraw may successfully submit this retained picture.
            return;
        };
        if !self.frame_pending {
            return;
        }
        self.frame_pending = false;
        self.metrics.submitted.fetch_add(1, Ordering::Relaxed);
        if let Some(previous) = self.last_submission.replace(submitted_at) {
            self.metrics
                .submit_interval
                .record(submitted_at.saturating_duration_since(previous));
        }
        if let Some(frame) = &self.frame {
            self.metrics
                .receive_to_submit
                .record(submitted_at.saturating_duration_since(frame.received_at));
            self.metrics.record_source_stamp(frame);
        }
    }

    fn exit_game(&mut self) {
        self.game_bindings.clear();
        self.game_wheel_deadlines.fill(None);
        let timestamp = self.timestamp();
        if self
            .game
            .as_mut()
            .and_then(|game| game.state.take())
            .is_some()
        {
            // The device releases its delivered contacts. Queued game states
            // may be ahead of it, especially during a look recenter.
            if !self.input.cancel_game(timestamp) {
                self.failed = true;
                let _ = self.stop.send(true);
            }
            if let Some(window) = &self.window {
                let _ = window.set_cursor_grab(CursorGrabMode::None);
                window.set_cursor_visible(true);
            }
            self.update_title();
        }
    }

    fn enter_game(&mut self) {
        self.release();
        let Some(game) = &mut self.game else { return };
        if game.calibration.is_some() {
            return;
        }
        let result = (|| -> anyhow::Result<GameState> {
            let mut state = GameState::new(game.profile.clone())?;
            let (width, height) = self
                .display_size
                .ok_or_else(|| anyhow::anyhow!("waiting for phone video"))?;
            state.set_aspect(f64::from(width) / f64::from(height))?;
            let window = self
                .window
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("waiting for window"))?;
            // Confined cursors hit a screen edge; aiming requires a true lock.
            window.set_cursor_grab(CursorGrabMode::Locked)?;
            window.set_cursor_visible(false);
            Ok(state)
        })();
        match result {
            Ok(state) => game.state = Some(state),
            Err(error) => {
                tracing::warn!(%error, "cannot enter game mode; calibrate with F9 and use a compositor with pointer lock")
            }
        }
        self.update_title();
    }

    fn game_key(&mut self, code: KeyCode, pressed: bool) -> bool {
        if self.game.is_none() {
            return false;
        }
        match code {
            KeyCode::F8 => {
                if pressed {
                    if self.game_active() {
                        self.exit_game();
                    } else {
                        self.enter_game();
                    }
                }
                true
            }
            KeyCode::F9 => {
                if pressed {
                    self.release();
                    if let Some(game) = &mut self.game {
                        game.start_calibration();
                    }
                    self.update_title();
                }
                true
            }
            KeyCode::F10 => {
                if pressed {
                    self.release();
                    if let Some(game) = &mut self.game {
                        game.start_target_calibration();
                    }
                    self.update_title();
                }
                true
            }
            KeyCode::Escape
                if self.game_active()
                    || self.game.as_ref().is_some_and(|g| g.calibration.is_some()) =>
            {
                if pressed {
                    self.exit_game();
                    if let Some(game) = &mut self.game {
                        game.calibration = None;
                    }
                    self.update_title();
                }
                true
            }
            KeyCode::F1 | KeyCode::F2
                if self.game_active()
                    && self.game.as_ref().is_some_and(|g| {
                        bindings::resolve(&g.profile.bindings, Input::Key(code)).is_none()
                    }) =>
            {
                self.exit_game();
                false
            }
            _ if self.game.as_ref().is_some_and(|g| g.calibration.is_some()) => {
                if pressed {
                    if let Some(game) = &mut self.game {
                        game.select_calibration_key(code);
                    }
                    self.update_title();
                }
                true
            }
            _ if self.game_active() => {
                self.game_input(Input::Key(code), pressed);
                true
            }
            _ => self.game.as_ref().is_some_and(|g| g.calibration.is_some()),
        }
    }

    fn game_action(&mut self, action: Action, pressed: bool) {
        let timestamp = self.timestamp();
        if let Some(state) = self.game.as_mut().and_then(|game| game.state.as_mut()) {
            let batch = state.key(action, pressed, self.rotation, timestamp);
            self.send_game(batch);
        }
    }

    fn game_input(&mut self, input: Input, pressed: bool) {
        let transition = self.game.as_ref().and_then(|game| {
            self.game_bindings
                .event(&game.profile.bindings, input, pressed)
        });
        if let Some((action, pressed)) = transition {
            self.game_action(action, pressed);
        }
    }

    fn game_mouse(&mut self, button: MouseButton, pressed: bool) -> bool {
        if self.game_active() {
            self.game_input(Input::Mouse(button), pressed);
            return true;
        }
        if self.game.as_ref().is_some_and(|g| g.calibration.is_some()) {
            if pressed {
                let selecting = self.game.as_ref().is_some_and(|g| {
                    matches!(
                        g.calibration,
                        Some(crate::game_ui::Calibration::SelectTarget)
                    )
                });
                if selecting {
                    if let Some(game) = &mut self.game {
                        game.select_calibration_input(Input::Mouse(button));
                    }
                } else if button == MouseButton::Left
                    && let Some((x, y)) = self.position(false)
                    && let Some(game) = &mut self.game
                    && let Err(error) = game.calibrate(x, y)
                {
                    tracing::warn!(%error, "calibration failed");
                }
                self.update_title();
            }
            return true;
        }
        false
    }

    fn game_wheel(&mut self, x: f64, y: f64) -> bool {
        if !self.game_active() && self.game.as_ref().is_none_or(|g| g.calibration.is_none()) {
            return false;
        }
        for (i, delta) in [y, -y, -x, x].into_iter().enumerate() {
            if !delta.is_finite() || delta <= 0.0 {
                continue;
            }
            let direction = [
                WheelDirection::Up,
                WheelDirection::Down,
                WheelDirection::Left,
                WheelDirection::Right,
            ][i];
            let input = Input::Wheel(direction);
            if self.game_active() {
                self.game_input(input, true);
                // A wheel has no release event. Bound taps last 30 ms, with
                // repeated ticks extending that hold rather than queuing taps.
                self.game_wheel_deadlines[i] = Some(self.timestamp().saturating_add(30_000_000));
            } else if let Some(game) = &mut self.game {
                game.select_calibration_input(input);
                self.update_title();
                break;
            }
        }
        true
    }

    fn advance_game(&mut self) -> Option<Instant> {
        let timestamp = self.timestamp();
        if !self.game_active() {
            return None;
        }
        for i in 0..4 {
            if self.game_wheel_deadlines[i].is_some_and(|at| at <= timestamp) {
                self.game_wheel_deadlines[i] = None;
                let direction = [
                    WheelDirection::Up,
                    WheelDirection::Down,
                    WheelDirection::Left,
                    WheelDirection::Right,
                ][i];
                self.game_input(Input::Wheel(direction), false);
            }
        }
        let state = self.game.as_mut()?.state.as_mut()?;
        let batch = state.expire_idle_movement(timestamp);
        let deadline = state
            .movement_deadline()
            .into_iter()
            .chain(self.game_wheel_deadlines.iter().flatten().copied())
            .min();
        self.send_game(batch);
        deadline.and_then(|at| self.clock.checked_add(Duration::from_nanos(at)))
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
                        .with_inner_size(LogicalSize::new(400.0, 918.0))
                        .with_min_inner_size(LogicalSize::new(180.0, 240.0)),
                )?,
            );
            let renderer = pollster::block_on(Renderer::new(window.clone(), self.presentation))?;
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
                self.update_home_visual();
                self.update_title();
            }
            AppEvent::FrameReady => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            AppEvent::OrientationChanged(orientation) => {
                if (1..=4).contains(&orientation) && u32::from(self.orientation) != orientation {
                    self.orientation = orientation as u8;
                    self.apply_view();
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
                self.update_home_visual();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                self.home_armed = false;
                self.update_home_visual();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::Resized(size) => {
                self.home_armed = false;
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
                self.update_home_visual();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                let fresh = self.latest.take();
                if let Some(fresh) = fresh {
                    let geometry_changed = self.frame.as_ref().is_none_or(|previous| {
                        (previous.width, previous.height) != (fresh.width, fresh.height)
                    });
                    self.frame = Some(fresh);
                    self.frame_pending = true;
                    if geometry_changed {
                        self.apply_view();
                    }
                }
                if let (Some(renderer), Some(frame)) = (&mut self.renderer, &self.frame) {
                    match renderer.render(frame) {
                        Ok(sample) => self.record_render(sample),
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
                if self.game_active() {
                    return;
                }
                self.pointer = position;
                self.update_home_visual();
                if self.focused
                    && self.connected
                    && self.wheel.is_none()
                    && let Some((x, y)) = self
                        .position(true)
                        .and_then(|(x, y)| normalized_position(x, y, self.rotation))
                {
                    let event = self.controls.touch_move(x, y, self.timestamp());
                    self.send(event);
                }
            }
            WindowEvent::CursorLeft { .. } => {
                if self.game_active() {
                    return;
                }
                self.cancel_touch();
                self.home_armed = false;
                self.pointer = PhysicalPosition::new(-1.0, -1.0);
                self.update_home_visual();
            }
            WindowEvent::MouseInput { state, button, .. } if self.focused && self.connected => {
                if self.game_mouse(button, state.is_pressed()) {
                    return;
                }
                if button != MouseButton::Left {
                    return;
                }
                let position = self
                    .position(false)
                    .and_then(|(x, y)| normalized_position(x, y, self.rotation));
                self.pointer_button(state == ElementState::Pressed, self.home_hit(), position);
                self.update_home_visual();
            }
            WindowEvent::MouseWheel { delta, .. } if self.focused && self.connected => {
                let (x, y) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (f64::from(x), f64::from(y)),
                    MouseScrollDelta::PixelDelta(p) => (p.x / 50.0, p.y / 50.0),
                };
                if self.game_wheel(x, y) {
                    return;
                }
                let lines = y;
                self.pending_scroll = (self.pending_scroll + lines).clamp(-4.0, 4.0);
                self.start_scroll();
            }
            WindowEvent::KeyboardInput { event, .. }
                if self.focused && self.connected && !event.repeat =>
            {
                if let PhysicalKey::Code(code) = event.physical_key {
                    let character = match &event.logical_key {
                        Key::Character(text) => {
                            let mut chars = text.chars();
                            let first = chars.next();
                            if chars.next().is_none() { first } else { None }
                        }
                        _ => None,
                    };
                    self.key_event(code, event.state.is_pressed(), character);
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if !self.focused || !self.connected {
            return;
        }
        if let DeviceEvent::MouseMotion { delta: (dx, dy) } = event {
            let timestamp = self.timestamp();
            if let Some(state) = self.game.as_mut().and_then(|game| game.state.as_mut()) {
                let batch = state.motion(dx, dy, self.rotation, timestamp);
                self.send_game(batch);
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.advance_scroll(Instant::now());
        let game_deadline = self.advance_game();
        event_loop.set_control_flow(
            self.wheel
                .as_ref()
                .map(|(_, _, at)| *at)
                .into_iter()
                .chain(game_deadline)
                .min()
                .map_or(ControlFlow::Wait, ControlFlow::WaitUntil),
        );
    }
}

/// Preserve the displayed short edge across rotation and reserve the footer.
/// Monitor limits keep a portrait-to-landscape flip on screen when possible.
fn fitted_window_size(
    current: (f64, f64),
    display: (u32, u32),
    limit: Option<(f64, f64)>,
) -> (f64, f64) {
    let footer = 48.0;
    let short = current.0.min((current.1 - footer).max(1.0)).max(180.0);
    let aspect_scale = short / f64::from(display.0.min(display.1).max(1));
    let mut width = f64::from(display.0) * aspect_scale;
    let mut height = f64::from(display.1) * aspect_scale;
    if let Some((max_width, max_height)) = limit {
        let scale = (max_width / width)
            .min((max_height - footer).max(1.0) / height)
            .min(1.0);
        width *= scale;
        height *= scale;
    }
    (width.max(1.0), height + footer)
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
        // Printable ASCII already carries host CapsLock in its logical value.
        // Toggling CapsLock again remotely would invert synthesized uppercase.
        CapsLock => return None,
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
        let metrics = Arc::new(Metrics::default());
        let mut app = App::new(
            Arc::new(LatestFrame::new()),
            Arc::new(InputBus::new(metrics.clone())),
            metrics,
            stop,
        );
        app.focused = true;
        app.connected = true;
        app
    }

    #[test]
    fn presentation_metrics_keep_timeout_retry_and_ignore_retained_redraws() {
        let mut app = app();
        let first = Instant::now();
        let sample = |submitted_at| RenderSample {
            submitted_at,
            upload: Duration::from_micros(200),
            surface_acquire: Duration::from_millis(2),
            total: Duration::from_millis(3),
        };
        app.frame_pending = true;
        app.record_render(sample(None));
        assert!(app.frame_pending);
        assert_eq!(app.metrics.submitted.load(Ordering::Relaxed), 0);
        app.record_render(sample(Some(first)));
        assert!(!app.frame_pending);
        assert_eq!(app.metrics.submitted.load(Ordering::Relaxed), 1);
        app.record_render(sample(Some(first + Duration::from_millis(2))));
        assert_eq!(app.metrics.submitted.load(Ordering::Relaxed), 1);
        app.frame_pending = true;
        app.record_render(sample(Some(first + Duration::from_millis(16))));
        assert_eq!(app.metrics.submitted.load(Ordering::Relaxed), 2);
        assert_eq!(app.metrics.submit_interval.samples(), 1);
        assert_eq!(app.metrics.submit_interval.mean_ms(), 16.0);
        assert_eq!(app.metrics.render_attempts.load(Ordering::Relaxed), 4);
        assert_eq!(app.metrics.surface_timeouts.load(Ordering::Relaxed), 1);
        assert_eq!(app.metrics.surface_acquire.samples(), 4);
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
    fn with_game() -> App {
        use iphone_mirror_rs::game::{CALIBRATION_TARGETS, Point, Profile};
        let mut app = app();
        let mut profile = Profile::default();
        for target in CALIBRATION_TARGETS.into_iter().filter(|&target| {
            !matches!(
                target,
                iphone_mirror_rs::game::Target::Sprint
                    | iphone_mirror_rs::game::Target::Button(Action::Custom(_))
            )
        }) {
            profile.set_point(target, Point { x: 0.5, y: 0.5 }).unwrap();
        }
        let state = GameState::new(profile.clone()).unwrap();
        app.game = Some(crate::game_ui::GameControls {
            path: "/unused".into(),
            profile,
            state: Some(state),
            calibration: None,
        });
        app
    }
    #[test]
    fn game_keys_do_not_type_on_phone_and_escape_releases_every_contact() {
        let mut app = with_game();
        app.key(KeyCode::KeyW, true);
        app.key(KeyCode::KeyC, true);
        app.key(KeyCode::KeyM, true);
        app.game_action(Action::Fire, true);
        app.key(KeyCode::KeyT, true); // Unbound keys must not leak into chat.
        let sent = events(&app);
        assert!(
            sent.iter()
                .all(|event| matches!(event, HidEvent::Touch { .. }))
        );
        app.key(KeyCode::Escape, true);
        assert!(!app.game_active());
        let sent = events(&app);
        assert!(matches!(sent.as_slice(), [HidEvent::ReleaseTouches { .. }]));
        app.release();
        assert!(events(&app).is_empty());
    }
    #[test]
    fn game_rotation_and_focus_cleanup_disable_capture() {
        for rotate in [false, true] {
            let mut app = with_game();
            app.key(KeyCode::KeyW, true);
            app.game_action(Action::Aim, true);
            events(&app);
            if rotate {
                app.set_rotation(270);
            } else {
                app.release();
            }
            assert!(!app.game_active());
            assert!(matches!(
                events(&app).as_slice(),
                [HidEvent::ReleaseTouches { .. }]
            ));
        }
    }

    #[test]
    fn either_shift_sprints_until_both_release_and_exit_clears_modifiers() {
        let mut app = with_game();
        app.key(KeyCode::KeyW, true);
        events(&app);
        app.key(KeyCode::ShiftLeft, true);
        assert_eq!(events(&app).len(), 1);
        app.key(KeyCode::ShiftRight, true);
        app.key(KeyCode::ShiftLeft, false);
        assert!(events(&app).is_empty());
        app.key(KeyCode::ShiftRight, false);
        assert_eq!(events(&app).len(), 1);
        app.key(KeyCode::ShiftLeft, true);
        events(&app);
        app.key(KeyCode::Escape, true);
        assert!(app.game_bindings.is_empty());
        assert!(matches!(
            events(&app).as_slice(),
            [HidEvent::ReleaseTouches { .. }]
        ));
    }

    #[test]
    fn custom_mouse_aliases_do_not_release_a_still_held_keyboard_action() {
        let mut app = with_game();
        app.game
            .as_mut()
            .unwrap()
            .profile
            .bindings
            .push(bindings::Binding::parse("mouse.Back", "reload").unwrap());
        app.key(KeyCode::KeyR, true);
        assert_eq!(events(&app).len(), 1);
        assert!(app.game_mouse(MouseButton::Back, true));
        app.key(KeyCode::KeyR, false);
        assert!(events(&app).is_empty());
        app.game_mouse(MouseButton::Back, false);
        assert_eq!(events(&app).len(), 1);
        assert!(app.game_bindings.is_empty());
    }

    #[test]
    fn wheel_taps_expire_and_escape_discards_pending_releases() {
        let mut app = with_game();
        app.game
            .as_mut()
            .unwrap()
            .profile
            .bindings
            .push(bindings::Binding::parse("wheel.Up", "fire").unwrap());
        assert!(app.game_wheel(0.0, 1.0));
        assert_eq!(events(&app).len(), 1);
        assert!(app.advance_game().is_some());
        app.game_wheel_deadlines[0] = Some(0);
        app.advance_game();
        assert_eq!(events(&app).len(), 1);
        assert!(app.game_bindings.is_empty());
        app.game_mouse(MouseButton::Left, true);
        events(&app);
        app.game_wheel(0.0, 1.0);
        assert!(events(&app).is_empty());
        app.game_wheel_deadlines[0] = Some(0);
        app.advance_game();
        assert!(events(&app).is_empty()); // Left mouse still holds fire.
        app.game_wheel(0.0, 1.0);
        app.key(KeyCode::Escape, true);
        assert!(app.game_wheel_deadlines.iter().all(Option::is_none));
        assert!(app.game_bindings.is_empty());
        assert!(matches!(
            events(&app).as_slice(),
            [HidEvent::ReleaseTouches { .. }]
        ));
        assert!(app.advance_game().is_none());
        assert!(events(&app).is_empty());
    }

    #[test]
    fn f10_accepts_mouse_selection_without_sending_a_phone_click() {
        let mut app = with_game();
        app.key(KeyCode::F10, true);
        events(&app); // Initial touch reset.
        assert!(app.game_mouse(MouseButton::Other(9), true));
        assert!(matches!(
            app.game.as_ref().unwrap().calibration,
            Some(crate::game_ui::Calibration::Single(_, Some(_)))
        ));
        assert!(events(&app).is_empty());
        app.key(KeyCode::Escape, true);
        assert!(app.game.as_ref().unwrap().calibration.is_none());
    }

    #[test]
    fn idle_game_timer_releases_movement_without_waiting_for_another_key() {
        let mut app = with_game();
        app.key(KeyCode::KeyW, true);
        app.key(KeyCode::KeyW, false);
        events(&app);
        assert!(app.advance_game().is_some());
        app.clock -= Duration::from_millis(200);
        assert!(app.advance_game().is_none());
        assert!(matches!(
            events(&app).as_slice(),
            [HidEvent::Touch {
                phase: TouchPhase::End,
                ..
            }]
        ));
        assert!(app.game_active());
    }

    #[test]
    fn targeted_calibration_exits_capture_and_consumes_binding_keys_locally() {
        let mut app = with_game();
        app.key(KeyCode::KeyW, true);
        events(&app);
        app.key(KeyCode::F10, true);
        assert!(!app.game_active());
        assert!(matches!(
            events(&app).as_slice(),
            [HidEvent::ReleaseTouches { .. }]
        ));
        app.key(KeyCode::KeyB, true);
        app.key(KeyCode::KeyB, false);
        assert!(events(&app).is_empty());
        assert!(matches!(
            app.game.as_ref().and_then(|g| g.calibration.as_ref()),
            Some(crate::game_ui::Calibration::Single(
                iphone_mirror_rs::game::Target::Button(Action::SecondaryGadget),
                None
            ))
        ));
        app.key(KeyCode::Escape, true);
        assert!(app.game.as_ref().is_some_and(|g| g.calibration.is_none()));
        assert!(events(&app).is_empty());
    }
    #[test]
    fn home_exits_game_before_dispatching_phone_button() {
        let mut app = with_game();
        app.key(KeyCode::KeyW, true);
        events(&app);
        app.key(KeyCode::F1, true);
        assert!(matches!(
            events(&app).as_slice(),
            [
                HidEvent::ReleaseTouches { .. },
                HidEvent::Home { pressed: true }
            ]
        ));
        assert!(!app.game_active());
    }
    #[test]
    fn escape_discards_queued_game_gestures_before_normal_input() {
        let mut app = with_game();
        app.key(KeyCode::KeyW, true);
        app.game_action(Action::Fire, true);
        for _ in 0..20 {
            let batch = app
                .game
                .as_mut()
                .unwrap()
                .state
                .as_mut()
                .unwrap()
                .motion(100.0, 0.0, 0, 1);
            app.send_game(batch);
        }
        app.key(KeyCode::Escape, true);
        app.key(KeyCode::F1, true);
        assert!(matches!(
            events(&app).as_slice(),
            [
                HidEvent::ReleaseTouches { .. },
                HidEvent::Home { pressed: true }
            ]
        ));
    }
    #[test]
    fn rotation_releases_contacts_and_cancels_old_scroll_coordinates() {
        let mut app = app();
        begin_wheel(&mut app);
        events(&app);
        app.home_armed = true;
        app.set_rotation(270);
        assert_eq!(app.rotation, 270);
        assert!(!app.home_armed);
        assert!(app.wheel.is_none());
        assert!(matches!(
            events(&app).as_slice(),
            [HidEvent::Touch {
                phase: TouchPhase::End,
                ..
            }]
        ));
        app.set_rotation(270);
        assert!(events(&app).is_empty());
    }

    #[test]
    fn fitting_rotates_video_dimensions_without_rotating_footer() {
        let landscape = fitted_window_size((400.0, 918.0), (2576, 1184), None);
        assert!((landscape.0 - 870.270270).abs() < 0.001);
        assert_eq!(landscape.1, 448.0);
        let portrait = fitted_window_size(landscape, (1184, 2576), None);
        assert_eq!(portrait.0, 400.0);
        assert!((portrait.1 - 918.270270).abs() < 0.001);
        let limited = fitted_window_size((1000.0, 1400.0), (2576, 1184), Some((900.0, 700.0)));
        assert!(limited.0 <= 900.0 && limited.1 <= 700.0);
        assert!((limited.0 / (limited.1 - 48.0) - 2576.0 / 1184.0).abs() < 1e-10);
    }

    #[test]
    fn home_button_click_sends_a_complete_home_tap_without_touch() {
        let mut app = app();
        app.pointer_button(true, true, None);
        assert!(events(&app).is_empty());
        app.pointer_button(false, true, None);
        assert_eq!(
            events(&app),
            [
                HidEvent::Home { pressed: true },
                HidEvent::Home { pressed: false }
            ]
        );
    }

    #[test]
    fn home_button_cancels_on_release_outside_or_focus_loss() {
        let mut app = app();
        app.pointer_button(true, true, None);
        app.pointer_button(false, false, None);
        assert!(events(&app).is_empty());
        app.pointer_button(true, true, None);
        app.release();
        app.pointer_button(false, true, None);
        assert!(events(&app).is_empty());
    }

    #[test]
    fn drag_ending_on_home_does_not_activate_it() {
        let mut app = app();
        app.pointer_button(true, false, Some((1000, 2000)));
        app.pointer_button(false, true, None);
        let events = events(&app);
        assert_eq!(events.len(), 2);
        assert!(
            events
                .iter()
                .all(|event| matches!(event, HidEvent::Touch { .. }))
        );
    }

    #[test]
    fn home_button_interrupts_wheel_and_preserves_held_f1() {
        let mut app = app();
        begin_wheel(&mut app);
        events(&app);
        app.pointer_button(true, true, None);
        assert!(matches!(
            events(&app).as_slice(),
            [HidEvent::Touch {
                phase: TouchPhase::End,
                ..
            }]
        ));
        app.key(KeyCode::F1, true);
        events(&app);
        app.pointer_button(false, true, None);
        assert!(events(&app).is_empty());
        app.key(KeyCode::F1, false);
        assert_eq!(events(&app), [HidEvent::Home { pressed: false }]);
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

    #[test]
    fn virtual_colon_gets_implicit_shift_and_releases_press_time_chord() {
        let mut app = app();
        app.key_event(KeyCode::Semicolon, true, Some(':'));
        let pressed = events(&app);
        let Some(HidEvent::Keyboard(report)) = pressed.last() else {
            panic!("keyboard press");
        };
        assert_ne!(report[1 + 51 / 8] & (1 << (51 % 8)), 0);
        assert_ne!(report[1 + 225 / 8] & (1 << (225 % 8)), 0);
        // Release event may carry the unshifted logical key after layout changes.
        app.key_event(KeyCode::Semicolon, false, Some(';'));
        let released = events(&app);
        let Some(HidEvent::Keyboard(report)) = released.last() else {
            panic!("keyboard release");
        };
        assert!(report[1..31].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn implicit_shift_does_not_release_physical_shift_or_other_implicit_owner() {
        let mut app = app();
        app.key(KeyCode::ShiftLeft, true);
        app.key_event(KeyCode::Semicolon, true, Some(':'));
        app.key_event(KeyCode::Digit1, true, Some('!'));
        events(&app);
        app.key_event(KeyCode::Semicolon, false, None);
        app.key(KeyCode::ShiftLeft, false);
        let released = events(&app);
        let Some(HidEvent::Keyboard(report)) = released.last() else {
            panic!("keyboard release");
        };
        assert_ne!(report[1 + 225 / 8] & (1 << (225 % 8)), 0);
        app.key_event(KeyCode::Digit1, false, None);
        let released = events(&app);
        let Some(HidEvent::Keyboard(report)) = released.last() else {
            panic!("keyboard release");
        };
        assert!(report[1..31].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn caps_lock_is_applied_by_host_character_not_toggled_on_phone() {
        let mut app = app();
        app.key(KeyCode::CapsLock, true);
        app.key(KeyCode::CapsLock, false);
        assert!(events(&app).is_empty());
        app.key_event(KeyCode::KeyA, true, Some('A'));
        let pressed = events(&app);
        let Some(HidEvent::Keyboard(report)) = pressed.last() else {
            panic!("keyboard press");
        };
        assert_ne!(report[1] & (1 << 4), 0);
        assert_ne!(report[1 + 225 / 8] & (1 << (225 % 8)), 0);
        assert_eq!(report[1 + 57 / 8] & (1 << (57 % 8)), 0);
    }
}
