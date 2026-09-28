//! Window-facing game controls; the touch state machine lives in the library.
use std::path::PathBuf;

use anyhow::{Context, Result};
use iphone_mirror_rs::game::{Action, CALIBRATION_TARGETS, GameState, Point, Profile, Target};
use winit::keyboard::KeyCode;

pub struct GameControls {
    pub path: PathBuf,
    pub profile: Profile,
    pub state: Option<GameState>,
    pub calibration: Option<(usize, Profile)>,
}

impl GameControls {
    pub fn load(path: PathBuf) -> Result<Self> {
        let profile = if path.exists() {
            Profile::load(&path).context("cannot read game profile")?
        } else {
            Profile::default()
        };
        Ok(Self {
            path,
            profile,
            state: None,
            calibration: None,
        })
    }

    pub fn title(&self) -> String {
        if let Some((index, _)) = &self.calibration {
            format!(
                "{}/{} CLICK {} - ESC CANCEL",
                index + 1,
                CALIBRATION_TARGETS.len(),
                target_label(CALIBRATION_TARGETS[*index])
            )
        } else if self.state.is_some() {
            "GAME - ESC RELEASES MOUSE - F8 EXITS".into()
        } else if self.profile.calibrated() {
            "F8 PLAY - F9 CALIBRATE".into()
        } else {
            "F9 CALIBRATE GAME CONTROLS".into()
        }
    }

    pub fn start_calibration(&mut self) {
        self.calibration = Some((0, self.profile.clone()));
    }

    /// Calibration consumes clicks locally. No touch reaches the phone.
    pub fn calibrate(&mut self, x: f64, y: f64) -> Result<()> {
        let Some((index, profile)) = &mut self.calibration else {
            return Ok(());
        };
        profile.set_point(CALIBRATION_TARGETS[*index], Point { x, y })?;
        if *index + 1 < CALIBRATION_TARGETS.len() {
            *index += 1;
            return Ok(());
        }
        // Save first: a failed write keeps the pending calibration available.
        profile
            .save(&self.path)
            .context("cannot save game calibration")?;
        self.profile = profile.clone();
        self.calibration = None;
        Ok(())
    }
}

fn target_label(target: Target) -> &'static str {
    match target {
        Target::Joystick => "joystick center",
        Target::Look => "empty aiming area",
        Target::Button(action) => match action {
            Action::LeanLeft => "lean left (Q)",
            Action::LeanRight => "lean right (E)",
            Action::Reload => "reload (R)",
            Action::Interact => "interact (F)",
            Action::Melee => "melee (V)",
            Action::Grenade => "grenade (G)",
            Action::Primary => "primary weapon (1)",
            Action::Secondary => "secondary weapon (2)",
            Action::Crouch => "crouch (C)",
            Action::Fire => "fire (left mouse)",
            Action::Aim => "aim down sights (right mouse)",
            _ => "movement joystick center",
        },
    }
}

pub fn action(code: KeyCode) -> Option<Action> {
    Some(match code {
        KeyCode::KeyW => Action::Up,
        KeyCode::KeyA => Action::Left,
        KeyCode::KeyS => Action::Down,
        KeyCode::KeyD => Action::Right,
        KeyCode::KeyQ => Action::LeanLeft,
        KeyCode::KeyE => Action::LeanRight,
        KeyCode::KeyR => Action::Reload,
        KeyCode::KeyF => Action::Interact,
        KeyCode::KeyV => Action::Melee,
        KeyCode::KeyG => Action::Grenade,
        KeyCode::Digit1 => Action::Primary,
        KeyCode::Digit2 => Action::Secondary,
        KeyCode::KeyC => Action::Crouch,
        _ => return None,
    })
}
