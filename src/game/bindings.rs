//! Persistent input overrides and alias-safe, allocation-free event dispatch.
use super::{Action, CALIBRATION_TARGETS, GameError, Target};
use winit::{event::MouseButton, keyboard::KeyCode};

pub const MAX_BINDINGS: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Key(KeyCode),
    Mouse(MouseButton),
    Wheel(WheelDirection),
}

impl Input {
    pub fn reserved(self) -> bool {
        matches!(
            self,
            Self::Key(KeyCode::Escape | KeyCode::F8 | KeyCode::F9 | KeyCode::F10)
        )
    }

    pub fn name(self) -> String {
        match self {
            Self::Key(key) => format!("key.{key:?}"),
            Self::Mouse(MouseButton::Other(id)) => format!("mouse.Other{id}"),
            Self::Mouse(button) => format!("mouse.{button:?}"),
            Self::Wheel(direction) => format!("wheel.{direction:?}"),
        }
    }

    fn parse(name: &str) -> Result<Self, GameError> {
        let invalid = || GameError::InvalidProfile("invalid keyboard, mouse or wheel input");
        if let Some(key) = name.strip_prefix("key.") {
            return serde_json::from_str::<KeyCode>(&format!("\"{key}\""))
                .map(Self::Key)
                .map_err(|_| invalid());
        }
        if let Some(button) = name.strip_prefix("mouse.") {
            return Ok(Self::Mouse(match button {
                "Left" => MouseButton::Left,
                "Right" => MouseButton::Right,
                "Middle" => MouseButton::Middle,
                "Back" => MouseButton::Back,
                "Forward" => MouseButton::Forward,
                _ => MouseButton::Other(
                    button
                        .strip_prefix("Other")
                        .ok_or_else(invalid)?
                        .parse()
                        .map_err(|_| invalid())?,
                ),
            }));
        }
        if let Some(direction) = name.strip_prefix("wheel.") {
            return Ok(Self::Wheel(match direction {
                "Up" => WheelDirection::Up,
                "Down" => WheelDirection::Down,
                "Left" => WheelDirection::Left,
                "Right" => WheelDirection::Right,
                _ => return Err(invalid()),
            }));
        }
        Err(invalid())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub input: Input,
    pub action: Option<Action>,
}

impl Binding {
    pub fn parse(input: &str, action: &str) -> Result<Self, GameError> {
        let binding = Self {
            input: Input::parse(input)?,
            action: parse_action(action)?,
        };
        validate(&[binding])?;
        Ok(binding)
    }

    pub fn action_name(self) -> &'static str {
        match self.action {
            None => "none",
            Some(Action::Up) => "up",
            Some(Action::Down) => "down",
            Some(Action::Left) => "left",
            Some(Action::Right) => "right",
            Some(Action::Sprint) => "sprint",
            Some(action) => Target::Button(action).name(),
        }
    }
}

fn parse_action(name: &str) -> Result<Option<Action>, GameError> {
    Ok(match name {
        "none" => None,
        "up" => Some(Action::Up),
        "down" => Some(Action::Down),
        "left" => Some(Action::Left),
        "right" => Some(Action::Right),
        "sprint" => Some(Action::Sprint),
        _ => Some(
            CALIBRATION_TARGETS
                .iter()
                .find_map(|target| match target {
                    Target::Button(action) if target.name() == name => Some(*action),
                    _ => None,
                })
                .ok_or(GameError::InvalidProfile("unknown binding action"))?,
        ),
    })
}

pub fn validate(bindings: &[Binding]) -> Result<(), GameError> {
    if bindings.len() > MAX_BINDINGS {
        return Err(GameError::InvalidProfile("too many input bindings"));
    }
    for (i, binding) in bindings.iter().enumerate() {
        if binding.input.reserved() {
            return Err(GameError::InvalidProfile("Escape and F8-F10 are reserved"));
        }
        if binding
            .action
            .is_some_and(|a| a.index() >= super::ACTION_COUNT)
        {
            return Err(GameError::InvalidProfile("invalid custom action"));
        }
        if matches!(binding.input, Input::Wheel(_))
            && binding
                .action
                .is_some_and(|a| a.index() < 4 || a == Action::Sprint)
        {
            return Err(GameError::InvalidProfile(
                "wheel inputs cannot hold movement or sprint",
            ));
        }
        if bindings[..i].iter().any(|b| b.input == binding.input) {
            return Err(GameError::InvalidProfile("duplicate input binding"));
        }
    }
    Ok(())
}

pub fn default_action(code: KeyCode) -> Option<Action> {
    Some(match code {
        KeyCode::KeyW => Action::Up,
        KeyCode::KeyA => Action::Left,
        KeyCode::KeyS => Action::Down,
        KeyCode::KeyD => Action::Right,
        KeyCode::ShiftLeft | KeyCode::ShiftRight => Action::Sprint,
        KeyCode::KeyQ => Action::LeanLeft,
        KeyCode::KeyE => Action::LeanRight,
        KeyCode::KeyR => Action::Reload,
        KeyCode::KeyF => Action::Interact,
        KeyCode::KeyV => Action::Melee,
        KeyCode::KeyG => Action::Grenade,
        KeyCode::Digit1 => Action::Primary,
        KeyCode::Digit2 => Action::Secondary,
        KeyCode::KeyC => Action::Crouch,
        KeyCode::Space => Action::Vault,
        KeyCode::KeyX => Action::Rappel,
        KeyCode::KeyB => Action::SecondaryGadget,
        KeyCode::KeyM => Action::Mount,
        _ => return None,
    })
}

pub fn resolve(overrides: &[Binding], input: Input) -> Option<Action> {
    if let Some(binding) = overrides.iter().find(|b| b.input == input) {
        return binding.action;
    }
    match input {
        Input::Key(key) => default_action(key),
        Input::Mouse(MouseButton::Left) => Some(Action::Fire),
        Input::Mouse(MouseButton::Right) => Some(Action::Aim),
        _ => None,
    }
}

pub struct RuntimeBindings {
    held: Vec<(Input, Action)>,
}

impl RuntimeBindings {
    pub fn new() -> Self {
        Self {
            held: Vec::with_capacity(MAX_BINDINGS),
        }
    }

    pub fn clear(&mut self) {
        self.held.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// Emit only aggregate action transitions. A rejected touch is never retried
    /// by a second alias, and releasing one alias cannot lift another's hold.
    pub fn event(
        &mut self,
        overrides: &[Binding],
        input: Input,
        pressed: bool,
    ) -> Option<(Action, bool)> {
        let index = self.held.iter().position(|(i, _)| *i == input);
        if pressed {
            if index.is_some() || self.held.len() == MAX_BINDINGS {
                return None;
            }
            let action = resolve(overrides, input)?;
            let already_held = self.held.iter().any(|(_, a)| *a == action);
            self.held.push((input, action));
            (!already_held).then_some((action, true))
        } else {
            let (_, action) = self.held.swap_remove(index?);
            (!self.held.iter().any(|(_, a)| *a == action)).then_some((action, false))
        }
    }
}

impl Default for RuntimeBindings {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_keyboard_mouse_wheel_and_rejects_conflicts() {
        for input in [
            "key.Numpad7",
            "key.ControlRight",
            "key.F24",
            "key.Semicolon",
            "mouse.Middle",
            "mouse.Back",
            "mouse.Forward",
            "mouse.Other42",
            "wheel.Up",
            "wheel.Left",
        ] {
            let binding = Binding::parse(input, "mount").unwrap();
            assert_eq!(binding.input.name(), input);
            assert_eq!(binding.action, Some(Action::Mount));
        }
        for (input, action) in [
            ("key.Escape", "fire"),
            ("key.F8", "none"),
            ("key.Invalid", "fire"),
            ("mouse.Other65536", "fire"),
            ("wheel.Up", "up"),
            ("key.KeyT", "invalid"),
            ("key.KeyT", "custom64"),
        ] {
            assert!(Binding::parse(input, action).is_err());
        }
        let binding = Binding::parse("key.KeyT", "mount").unwrap();
        assert!(validate(&[binding, binding]).is_err());
    }

    #[test]
    fn aliases_repeat_releases_and_reset_preserve_aggregate_hold() {
        let overrides = [Binding::parse("mouse.Back", "reload").unwrap()];
        let mut runtime = RuntimeBindings::new();
        let capacity = runtime.held.capacity();
        let key = Input::Key(KeyCode::KeyR);
        let mouse = Input::Mouse(MouseButton::Back);
        assert_eq!(
            runtime.event(&overrides, key, true),
            Some((Action::Reload, true))
        );
        assert_eq!(runtime.event(&overrides, key, true), None);
        assert_eq!(runtime.event(&overrides, mouse, true), None);
        assert_eq!(runtime.event(&overrides, key, false), None);
        assert_eq!(
            runtime.event(&overrides, mouse, false),
            Some((Action::Reload, false))
        );
        assert_eq!(runtime.event(&overrides, mouse, false), None);
        runtime.event(&overrides, mouse, true);
        runtime.clear();
        assert!(runtime.is_empty());
        assert_eq!(runtime.event(&overrides, mouse, false), None);
        assert_eq!(runtime.held.capacity(), capacity);
    }

    #[test]
    fn remapping_and_unbinding_replace_defaults() {
        let bindings = [
            Binding::parse("key.KeyW", "custom0").unwrap(),
            Binding::parse("key.ArrowUp", "up").unwrap(),
            Binding::parse("mouse.Left", "none").unwrap(),
        ];
        assert_eq!(
            resolve(&bindings, Input::Key(KeyCode::KeyW)),
            Some(Action::Custom(0))
        );
        assert_eq!(
            resolve(&bindings, Input::Key(KeyCode::ArrowUp)),
            Some(Action::Up)
        );
        assert_eq!(resolve(&bindings, Input::Mouse(MouseButton::Left)), None);
        assert_eq!(
            resolve(&bindings, Input::Mouse(MouseButton::Right)),
            Some(Action::Aim)
        );
    }

    #[test]
    fn profile_roundtrip_keeps_custom_targets_sprint_and_input_overrides_distinct() {
        use super::super::{Point, Profile};
        let path =
            std::env::temp_dir().join(format!("mirror-bindings-{}.profile", uuid::Uuid::new_v4()));
        let mut profile = Profile::default();
        for (i, target) in CALIBRATION_TARGETS.into_iter().enumerate() {
            profile
                .set_point(
                    target,
                    Point {
                        x: i as f64 / 100.0,
                        y: if target == Target::Sprint { 0.1 } else { 0.7 },
                    },
                )
                .unwrap();
        }
        profile.bindings = vec![
            Binding::parse("key.KeyT", "custom63").unwrap(),
            Binding::parse("mouse.Other65535", "reload").unwrap(),
            Binding::parse("wheel.Right", "secondary").unwrap(),
        ];
        profile.save(&path).unwrap();
        let loaded = Profile::load(&path).unwrap();
        assert_eq!(loaded.bindings, profile.bindings);
        for target in CALIBRATION_TARGETS {
            assert_eq!(loaded.point(target), profile.point(target));
        }
        std::fs::remove_file(path).unwrap();
        assert!(Profile::parse("version=1\nbind.key.KeyT=fire\nbind.key.KeyT=aim").is_err());
        let mut state = super::super::GameState::new(loaded).unwrap();
        assert!(state.key(Action::Custom(255), true, 0, 0).error.is_some());
    }
}
