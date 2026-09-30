//! Window-facing game controls; the touch state machine lives in the library.
use std::path::PathBuf;

use anyhow::{Context, Result};
use iphone_mirror_rs::game::bindings::{self, Binding, Input};
use iphone_mirror_rs::game::{
    Action, CALIBRATION_TARGETS, GameState, Point, Profile, REQUIRED_TARGET_COUNT, Target,
};
use winit::keyboard::KeyCode;

pub struct GameControls {
    pub path: PathBuf,
    pub profile: Profile,
    pub state: Option<GameState>,
    pub calibration: Option<Calibration>,
}

pub enum Calibration {
    Sequence(usize, Box<Profile>),
    SelectTarget,
    Single(Target, Option<Binding>),
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
        if let Some(calibration) = &self.calibration {
            match calibration {
                Calibration::SelectTarget => {
                    "PRESS KEY OR MOUSE BUTTON / WHEEL - ESC CANCEL".into()
                }
                Calibration::Single(target, binding) => {
                    if let Some(binding) = binding {
                        return format!(
                            "CLICK HUD TARGET FOR {} - ESC CANCEL",
                            binding.input.name()
                        );
                    }
                    format!("CLICK {} - ESC CANCEL", target_label(*target))
                }
                Calibration::Sequence(index, _) => format!(
                    "{}/{} CLICK {} - ESC CANCEL",
                    index + 1,
                    REQUIRED_TARGET_COUNT,
                    target_label(CALIBRATION_TARGETS[*index])
                ),
            }
        } else if self.state.is_some() {
            "GAME - ESC RELEASES MOUSE - F8 EXITS".into()
        } else if self.profile.calibrated() {
            "F8 PLAY - F9 SETUP - F10 MAP INPUT".into()
        } else {
            "F9 CALIBRATE GAME CONTROLS".into()
        }
    }

    pub fn start_calibration(&mut self) {
        self.calibration = Some(Calibration::Sequence(0, Box::new(self.profile.clone())));
    }

    pub fn start_target_calibration(&mut self) {
        self.calibration = Some(Calibration::SelectTarget);
    }

    pub fn select_calibration_key(&mut self, code: KeyCode) {
        self.select_calibration_input(Input::Key(code));
    }

    pub fn select_calibration_input(&mut self, input: Input) {
        if input.reserved()
            || !matches!(
                self.calibration,
                Some(Calibration::SelectTarget | Calibration::Single(..))
            )
        {
            return;
        }
        let resolved = bindings::resolve(&self.profile.bindings, input);
        let (target, binding) = if let Some(action) = resolved {
            (
                match action {
                    Action::Up | Action::Down | Action::Left | Action::Right => Target::Joystick,
                    Action::Sprint => Target::Sprint,
                    _ => Target::Button(action),
                },
                None,
            )
        } else {
            let Some(index) = (0..iphone_mirror_rs::game::CUSTOM_TARGET_COUNT).find(|&i| {
                let action = Action::Custom(i as u8);
                self.profile.point(Target::Button(action)).is_none()
                    && !self
                        .profile
                        .bindings
                        .iter()
                        .any(|b| b.action == Some(action))
            }) else {
                tracing::warn!(
                    "all custom HUD targets are in use; reuse an existing action with a profile binding"
                );
                return;
            };
            let action = Action::Custom(index as u8);
            (
                Target::Button(action),
                Some(Binding {
                    input,
                    action: Some(action),
                }),
            )
        };
        self.calibration = Some(Calibration::Single(target, binding));
    }

    /// Calibration consumes clicks locally. No touch reaches the phone.
    pub fn calibrate(&mut self, x: f64, y: f64) -> Result<()> {
        let updated = match &mut self.calibration {
            Some(Calibration::Sequence(index, profile)) => {
                profile.set_point(CALIBRATION_TARGETS[*index], Point { x, y })?;
                if *index + 1 < REQUIRED_TARGET_COUNT {
                    *index += 1;
                    return Ok(());
                }
                profile.as_ref().clone()
            }
            Some(Calibration::Single(target, binding)) => {
                let mut profile = self.profile.clone();
                profile.set_point(*target, Point { x, y })?;
                if let Some(binding) = binding {
                    if let Some(existing) = profile
                        .bindings
                        .iter_mut()
                        .find(|b| b.input == binding.input)
                    {
                        *existing = *binding;
                    } else {
                        profile.bindings.push(*binding);
                    }
                }
                profile
            }
            _ => return Ok(()),
        };
        // Save first: a failed write keeps the pending calibration available.
        updated
            .save(&self.path)
            .context("cannot save game calibration")?;
        self.profile = updated;
        self.calibration = None;
        Ok(())
    }
}

fn target_label(target: Target) -> &'static str {
    match target {
        Target::Joystick => "joystick center",
        Target::Look => "empty aiming area",
        Target::Sprint => "forward sprint endpoint above joystick (Shift)",
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
            Action::Vault => "vault / climb (Space)",
            Action::Rappel => "rappel (X)",
            Action::SecondaryGadget => "second throwable / gadget (B)",
            Action::Mount => "mount (M)",
            _ => "movement joystick center",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arbitrary_keys_and_every_mouse_kind_calibrate_and_persist_independently() -> Result<()> {
        use bindings::WheelDirection;
        use winit::event::MouseButton;
        let path =
            std::env::temp_dir().join(format!("mirror-custom-{}.profile", uuid::Uuid::new_v4()));
        let mut game = GameControls::load(path.clone())?;
        let inputs = [
            Input::Key(KeyCode::KeyT),
            Input::Key(KeyCode::Numpad7),
            Input::Mouse(MouseButton::Middle),
            Input::Mouse(MouseButton::Back),
            Input::Mouse(MouseButton::Forward),
            Input::Mouse(MouseButton::Other(8)),
            Input::Wheel(WheelDirection::Up),
        ];
        for (i, input) in inputs.into_iter().enumerate() {
            game.start_target_calibration();
            game.select_calibration_input(input);
            let point = Point {
                x: 0.1 * i as f64,
                y: 0.4,
            };
            game.calibrate(point.x, point.y)?;
            let loaded = Profile::load(&path)?;
            let action = bindings::resolve(&loaded.bindings, input)
                .ok_or_else(|| anyhow::anyhow!("missing saved binding"))?;
            assert_eq!(action, Action::Custom(i as u8));
            assert_eq!(loaded.point(Target::Button(action)), Some(point));
        }
        game.start_target_calibration();
        game.select_calibration_input(Input::Key(KeyCode::Escape));
        assert!(matches!(game.calibration, Some(Calibration::SelectTarget)));
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn targeted_calibration_saves_only_the_selected_binding() -> Result<()> {
        let path =
            std::env::temp_dir().join(format!("mirror-target-{}.profile", uuid::Uuid::new_v4()));
        let mut game = GameControls::load(path.clone())?;
        game.profile
            .set_point(Target::Joystick, Point { x: 0.2, y: 0.7 })?;
        game.profile
            .set_point(Target::Button(Action::Grenade), Point { x: 0.4, y: 0.2 })?;
        for (key, target, point) in [
            (
                KeyCode::Space,
                Target::Button(Action::Vault),
                Point { x: 0.8, y: 0.6 },
            ),
            (
                KeyCode::KeyX,
                Target::Button(Action::Rappel),
                Point { x: 0.7, y: 0.5 },
            ),
            (
                KeyCode::KeyB,
                Target::Button(Action::SecondaryGadget),
                Point { x: 0.6, y: 0.4 },
            ),
            (KeyCode::ShiftLeft, Target::Sprint, Point { x: 0.2, y: 0.3 }),
            (
                KeyCode::KeyM,
                Target::Button(Action::Mount),
                Point { x: 0.8, y: 0.4 },
            ),
        ] {
            game.start_target_calibration();
            game.calibrate(0.9, 0.9)?; // No target selected: ignore click.
            assert!(matches!(game.calibration, Some(Calibration::SelectTarget)));
            game.select_calibration_key(key);
            game.calibrate(point.x, point.y)?;
            assert!(game.calibration.is_none());
            assert_eq!(Profile::load(&path)?.point(target), Some(point));
        }
        assert_eq!(
            game.profile.point(Target::Button(Action::Grenade)),
            Some(Point { x: 0.4, y: 0.2 })
        );
        game.start_target_calibration();
        game.select_calibration_key(KeyCode::ShiftRight);
        assert!(game.calibrate(0.2, 0.8).is_err());
        assert!(game.calibration.is_some());
        assert_eq!(
            Profile::load(&path)?.point(Target::Sprint),
            Some(Point { x: 0.2, y: 0.3 })
        );
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn new_bindings_are_independent_of_existing_grenade_and_interact() {
        assert_eq!(
            bindings::default_action(KeyCode::Space),
            Some(Action::Vault)
        );
        assert_eq!(
            bindings::default_action(KeyCode::KeyX),
            Some(Action::Rappel)
        );
        assert_eq!(
            bindings::default_action(KeyCode::KeyB),
            Some(Action::SecondaryGadget)
        );
        assert_eq!(bindings::default_action(KeyCode::KeyM), Some(Action::Mount));
        assert_eq!(
            bindings::default_action(KeyCode::KeyG),
            Some(Action::Grenade)
        );
        assert_eq!(
            bindings::default_action(KeyCode::KeyF),
            Some(Action::Interact)
        );
    }
}
